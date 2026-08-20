use rd_ast::{RdDocument, RdNode, RdPath, RdPathSegment, RdTag};
use rd_writer::WriteError;

use crate::source::{FileId, Span, TextRange};

use super::{
    OriginBuilder, OriginPathSegment, assert_paths_address_nodes, content_spans, span_for_path,
};
use crate::inherit::{DocumentationOrigin, InheritableContent, InheritanceTrace, ResolvedContent};
use crate::model::{ResolvedUsage, UsageContribution};
use crate::source::{SourceFile, Spanned};
use crate::tags::{InheritField, RCodeText, SourcedText, TagOrigin, TagValue};

fn span(start: u32) -> Span {
    Span::new(FileId::new(0), TextRange::new(start, start + 1))
}

fn resolve_public_path<'a>(document: &'a RdDocument, path: &RdPath) -> Option<&'a RdNode> {
    let first = path.segments().first()?;
    let mut node = match first {
        RdPathSegment::TopLevel(index) => document.nodes().get(*index)?,
        _ => return None,
    };
    let mut position = 1;
    while position < path.segments().len() {
        match &path.segments()[position] {
            RdPathSegment::Child(index) => {
                node = match node {
                    RdNode::Tagged(tagged) => tagged.children().get(*index)?,
                    RdNode::Group(group) => group.children().get(*index)?,
                    RdNode::Raw(raw) => raw.children().get(*index)?,
                    _ => return None,
                };
                position += 1;
            }
            RdPathSegment::Option => {
                let Some(RdPathSegment::Child(index)) = path.segments().get(position + 1) else {
                    return None;
                };
                node = match node {
                    RdNode::Tagged(tagged) => tagged.option()?.get(*index)?,
                    RdNode::Raw(raw) => raw.option()?.get(*index)?,
                    _ => return None,
                };
                position += 2;
            }
            RdPathSegment::Attribute(_)
            | RdPathSegment::AttributeValue
            | RdPathSegment::ListElement(_)
            | RdPathSegment::CharacterElement(_) => return Some(node),
            _ => return None,
        }
    }
    Some(node)
}

fn origin_for_node(map: &super::OriginMap, node: &RdNode) -> Span {
    let (id, _) = map
        .nodes
        .iter()
        .find(|(_, candidate)| *candidate == node)
        .expect("resolved node has an origin identity");
    *map.spans
        .get(id)
        .and_then(|spans| spans.first())
        .expect("resolved node has an origin span")
}

fn assert_writer_path_origin(document: &RdDocument, map: &super::OriginMap) {
    let error = rd_writer::Writer::new(rd_writer::WriterOptions::default())
        .write_document(document)
        .expect_err("fixture must produce a path-bearing writer error");
    let WriteError::Unserializable { path, .. } = &error else {
        panic!("expected an unserializable writer error, got {error:?}");
    };
    let containing_path = if matches!(path.segments().last(), Some(RdPathSegment::Option)) {
        RdPath::new(path.segments()[..path.segments().len() - 1].to_vec())
    } else {
        path.clone()
    };
    let resolved = resolve_public_path(document, &containing_path)
        .expect("canonical writer path resolves to a document node");
    let expected = origin_for_node(map, resolved);
    assert_eq!(
        span_for_path(map, path),
        Some(expected),
        "writer path {path:?}"
    );
}

#[test]
fn root_child_paths_do_not_alias_top_level_paths() {
    let mut builder = OriginBuilder::new();
    let node = builder.append_text("value");
    builder.record(node, &[span(10)]);
    let (_, map) = builder.materialize();
    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![
                RdPathSegment::Child(0),
                RdPathSegment::CharacterElement(1)
            ])
        ),
        None
    );
    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![
                RdPathSegment::TopLevel(0),
                RdPathSegment::CharacterElement(1)
            ])
        ),
        Some(span(10))
    );
}

#[test]
fn option_paths_descend_before_falling_back() {
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("=topic");
    let display = builder.text_child("label");
    let link = builder.tagged_child(RdTag::Link, Some(vec![option]), vec![display]);
    builder.record(option, &[span(11)]);
    builder.record(display, &[span(12)]);
    builder.add_root(link);
    let (_, map) = builder.materialize();
    let option_path = RdPath::new(vec![
        RdPathSegment::TopLevel(0),
        RdPathSegment::Option,
        RdPathSegment::Child(0),
        RdPathSegment::CharacterElement(2),
    ]);
    assert_eq!(span_for_path(&map, &option_path), Some(span(11)));
}

#[test]
fn writer_error_after_an_inserted_sibling_keeps_the_later_leaf_origin() {
    let mut builder = OriginBuilder::new();
    let first = builder.rcode_child("f()");
    let second = builder.rcode_child("g()");
    builder.record(first, &[span(50)]);
    builder.record(second, &[span(60)]);
    let usage = builder.tagged_child(RdTag::Usage, None, vec![first, second]);
    builder.add_root(usage);
    builder.insert_root_before(usage, RdNode::Text("\n".into()));
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(50), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(60),
        "diagnostics: {diagnostics:?}"
    );
}

#[test]
fn single_argument_tag_uses_canonical_child_index() {
    let mut builder = OriginBuilder::new();
    let first = builder.verb_child("first");
    let second = builder.verb_child("bad\r");
    builder.record(first, &[span(50)]);
    builder.record(second, &[span(60)]);
    let name = builder.tagged_child(RdTag::Name, None, vec![first, second]);
    builder.record(name, &[span(40)]);
    builder.add_root(name);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(40), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(60)
    );
}

#[test]
fn unsupported_option_path_falls_back_to_containing_tag() {
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("unused");
    let child = builder.verb_child("name");
    builder.record(option, &[span(71)]);
    let name = builder.tagged_child(RdTag::Name, Some(vec![option]), vec![child]);
    builder.record(name, &[span(70)]);
    builder.add_root(name);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(70), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(70),
        "a bare option path falls back to the containing tag"
    );
}

#[test]
fn an_item_carrying_an_option_reports_the_item() {
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("unused");
    let body = builder.text_child("entry");
    builder.record(option, &[span(61)]);
    builder.record(body, &[span(62)]);
    let item = builder.tagged_child(RdTag::Item, Some(vec![option]), vec![body]);
    builder.record(item, &[span(60)]);
    let itemize = builder.tagged_child(RdTag::Itemize, None, vec![item]);
    builder.record(itemize, &[span(59)]);
    builder.add_root(itemize);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(59), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(60),
        "the writer anchors this at the bare option, which resolves to its item"
    );
}

#[test]
fn bare_option_path_requires_an_existing_option() {
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("target");
    let with_option = builder.tagged_child(RdTag::Link, Some(vec![option]), Vec::new());
    let without_option = builder.tagged_child(RdTag::Link, None, Vec::new());
    builder.record(option, &[span(81)]);
    builder.record(with_option, &[span(82)]);
    builder.record(without_option, &[span(83)]);
    builder.add_root(with_option);
    builder.add_root(without_option);
    let (_, map) = builder.materialize();

    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![RdPathSegment::TopLevel(0), RdPathSegment::Option])
        ),
        Some(span(82))
    );
    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![RdPathSegment::TopLevel(1), RdPathSegment::Option])
        ),
        None
    );
}

#[test]
fn existing_node_without_an_origin_falls_back_to_its_ancestor() {
    let mut builder = OriginBuilder::new();
    let child = builder.text_child("value");
    let tag = builder.tagged_child(RdTag::Link, None, vec![child]);
    builder.record(tag, &[span(84)]);
    builder.add_root(tag);
    let (_, map) = builder.materialize();

    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![
                RdPathSegment::TopLevel(0),
                RdPathSegment::Child(0),
                RdPathSegment::CharacterElement(0),
            ])
        ),
        Some(span(84))
    );
}

#[test]
fn shared_node_occurrences_each_get_an_origin_path() {
    let mut builder = OriginBuilder::new();
    let shared = builder.text_child("bad]");
    let link = builder.tagged_child(RdTag::Link, Some(vec![shared]), vec![shared]);
    builder.record(shared, &[span(20)]);
    builder.record(link, &[span(10)]);
    builder.add_root(link);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(10), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(20)
    );
}

#[test]
fn malformed_writer_paths_do_not_resolve_a_prefix() {
    let mut builder = OriginBuilder::new();
    let child = builder.text_child("value");
    let tag = builder.tagged_child(RdTag::Link, None, vec![child]);
    builder.record(child, &[span(30)]);
    builder.record(tag, &[span(31)]);
    builder.add_root(tag);
    let (_, map) = builder.materialize();

    for path in [
        RdPath::new(vec![RdPathSegment::Child(0)]),
        RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Child(0),
            RdPathSegment::TopLevel(1),
        ]),
        RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Option,
            RdPathSegment::Option,
        ]),
        RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Option,
            RdPathSegment::CharacterElement(0),
        ]),
        RdPath::new(vec![RdPathSegment::TopLevel(0), RdPathSegment::Child(99)]),
        RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Option,
            RdPathSegment::Child(0),
        ]),
    ] {
        assert_eq!(span_for_path(&map, &path), None, "path {path:?}");
    }

    assert_eq!(
        span_for_path(
            &map,
            &RdPath::new(vec![
                RdPathSegment::TopLevel(0),
                RdPathSegment::CharacterElement(0),
            ])
        ),
        Some(span(31))
    );
}

#[test]
fn writer_paths_conform_to_the_canonical_origin_grammar() {
    let mut cases = Vec::new();

    // A top-level node.
    let mut builder = OriginBuilder::new();
    let root = builder.append_node(RdNode::group(Vec::new()));
    builder.record(root, &[span(100)]);
    cases.push(builder.materialize());

    // A flattened tagged child.
    let mut builder = OriginBuilder::new();
    let first = builder.verb_child("first");
    let second = builder.verb_child("bad\r");
    let tag = builder.tagged_child(RdTag::Name, None, vec![first, second]);
    builder.record(first, &[span(110)]);
    builder.record(second, &[span(111)]);
    builder.record(tag, &[span(112)]);
    builder.add_root(tag);
    cases.push(builder.materialize());

    // A group child.
    let mut builder = OriginBuilder::new();
    let target_text = builder.text_child("target");
    let target = builder.group_child(vec![target_text]);
    let body_text = builder.text_child("body");
    let body = builder.group_child(vec![body_text]);
    let conditional = builder.tagged_child(RdTag::IfDef, None, vec![target, body]);
    builder.record(target_text, &[span(120)]);
    builder.record(target, &[span(121)]);
    builder.record(body_text, &[span(122)]);
    builder.record(body, &[span(123)]);
    builder.record(conditional, &[span(124)]);
    builder.add_root(conditional);
    cases.push(builder.materialize());

    // An option child.
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("bad]");
    let display = builder.text_child("label");
    let link = builder.tagged_child(RdTag::Link, Some(vec![option]), vec![display]);
    builder.record(option, &[span(130)]);
    builder.record(display, &[span(131)]);
    builder.record(link, &[span(132)]);
    builder.add_root(link);
    cases.push(builder.materialize());

    // A nested option.
    let mut builder = OriginBuilder::new();
    let nested_option_text = builder.text_child("bad]");
    let nested = builder.tagged_child(RdTag::Emph, Some(vec![nested_option_text]), Vec::new());
    let display = builder.text_child("label");
    let link = builder.tagged_child(RdTag::Link, Some(vec![nested]), vec![display]);
    builder.record(nested_option_text, &[span(140)]);
    builder.record(nested, &[span(141)]);
    builder.record(display, &[span(142)]);
    builder.record(link, &[span(143)]);
    builder.add_root(link);
    cases.push(builder.materialize());

    // A bare option, which resolves to its containing tagged node.
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("unused");
    let child = builder.verb_child("name");
    let name = builder.tagged_child(RdTag::Name, Some(vec![option]), vec![child]);
    builder.record(option, &[span(150)]);
    builder.record(child, &[span(151)]);
    builder.record(name, &[span(152)]);
    builder.add_root(name);
    cases.push(builder.materialize());

    for (document, map) in &cases {
        assert_paths_address_nodes(map, document);
        assert_writer_path_origin(document, map);
    }
}

#[test]
fn invalid_option_content_resolves_to_the_option_child() {
    let mut builder = OriginBuilder::new();
    let option = builder.text_child("bad]");
    let display = builder.text_child("label");
    builder.record(option, &[span(81)]);
    let link = builder.tagged_child(RdTag::Link, Some(vec![option]), vec![display]);
    builder.record(link, &[span(80)]);
    builder.add_root(link);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(80), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(81)
    );
}

#[test]
fn wrong_kind_positional_child_resolves_to_that_child() {
    let mut builder = OriginBuilder::new();
    let first_text = builder.text_child("href");
    let first = builder.group_child(vec![first_text]);
    let wrong = builder.text_child("wrong kind");
    builder.record(wrong, &[span(91)]);
    let href = builder.tagged_child(RdTag::Href, None, vec![first, wrong]);
    builder.record(href, &[span(90)]);
    builder.add_root(href);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(90), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(91)
    );
}

#[test]
fn conditional_body_failure_resolves_to_the_body_group() {
    let mut builder = OriginBuilder::new();
    let target_text = builder.text_child("target\n");
    let target = builder.group_child(vec![target_text]);
    let body_text = builder.text_child("body");
    let body = builder.group_child(vec![body_text]);
    builder.record(body, &[span(101)]);
    let conditional = builder.tagged_child(RdTag::IfDef, None, vec![target, body]);
    builder.record(conditional, &[span(100)]);
    builder.add_root(conditional);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(100), &mut diagnostics).is_none()
    );
    assert_eq!(
        diagnostics
            .iter()
            .next()
            .expect("writer diagnostic")
            .primary
            .span,
        span(101)
    );
}

#[test]
fn markdown_link_option_nodes_keep_their_fragment_origin() {
    let conversion = crate::markdown_conversion::convert_markdown(
        &crate::markdown_conversion::test_support::value("[text][obj]"),
        &crate::markdown_conversion::test_support::context(),
    );
    let mut builder = OriginBuilder::new();
    let roots = builder.append_fragment(&conversion.fragment);
    let link = roots.first().copied().expect("link root");
    builder.add_root(link);
    let (document, map) = builder.materialize();
    assert_paths_address_nodes(&map, &document);
    assert!(map.paths.iter().any(|(path, id)| {
        path.contains(&OriginPathSegment::Option)
            && map.spans.get(id).is_some_and(|spans| !spans.is_empty())
    }));
}

#[test]
fn link_option_inline_markup_keeps_its_argument_edge() {
    let mut builder = OriginBuilder::new();
    let option_text = builder.text_child("target");
    let option_markup = builder.tagged_child(RdTag::Emph, None, vec![option_text]);
    let display = builder.text_child("label");
    let link = builder.tagged_child(RdTag::Link, Some(vec![option_markup]), vec![display]);
    builder.add_root(link);
    let (document, map) = builder.materialize();
    assert_paths_address_nodes(&map, &document);
}

#[test]
fn usage_contributions_keep_separate_origins() {
    let source = SourceFile::new(std::path::PathBuf::from("test.R"), " ".repeat(100));
    let value = |start: u32, text: &str| TagValue {
        value: RCodeText::new(SourcedText::from_lines(
            &source,
            &[Span::new(
                FileId::new(0),
                TextRange::new(start, start + text.len() as u32),
            )],
            crate::tags::NormalizeHead::Intro,
        )),
        origin: TagOrigin::Explicit {
            name: Spanned {
                value: "usage".into(),
                span: span(start),
            },
            value_span: span(start + 1),
            full_span: span(start + 2),
        },
    };
    let contribution = |start: u32, text: &str| UsageContribution {
        block: crate::model::BlockRef {
            file: FileId::new(0),
            block: crate::arity_adapter::BlockId::new(start),
        },
        block_span: span(start + 20),
        object: None,
        method: None,
        usage: ResolvedUsage::Explicit(value(start, text)),
    };
    let mut builder = OriginBuilder::new();
    let usage = super::super::usage::lower(
        &[contribution(70, "f()"), contribution(80, "g()")],
        &mut builder,
        &mut crate::diagnostic::Diagnostics::new(),
    )
    .expect("usage section");
    let contribution_nodes = builder.arena[usage.0].children.clone();
    builder.add_root(usage);
    let (_, map) = builder.materialize();
    for (node, expected) in [
        (contribution_nodes[1], span(70)),
        (contribution_nodes[3], span(80)),
    ] {
        assert!(map.spans[&node].contains(&expected));
        let index = if node == contribution_nodes[1] { 1 } else { 3 };
        let path = RdPath::new(vec![
            RdPathSegment::TopLevel(0),
            RdPathSegment::Child(index),
        ]);
        assert_eq!(
            span_for_path(&map, &path),
            Some(expected),
            "writer path {path:?} must resolve to node {node:?}"
        );
    }
}

#[test]
fn argument_level_writer_failure_falls_back_to_the_usage_origin() {
    let invalid = r#"g(x = r"(unclosed)"#;
    let source = SourceFile::new(
        std::path::PathBuf::from("test.R"),
        format!("{}f()\n{}{}", " ".repeat(70), " ".repeat(6), invalid),
    );
    let value = |start: u32, text: &str| TagValue {
        value: RCodeText::new(SourcedText::from_lines(
            &source,
            &[Span::new(
                FileId::new(0),
                TextRange::new(start, start + text.len() as u32),
            )],
            crate::tags::NormalizeHead::Intro,
        )),
        origin: TagOrigin::Explicit {
            name: Spanned {
                value: "usage".into(),
                span: span(start),
            },
            value_span: span(start + 1),
            full_span: span(start + 2),
        },
    };
    let contribution = |start: u32, text: &str| UsageContribution {
        block: crate::model::BlockRef {
            file: FileId::new(0),
            block: crate::arity_adapter::BlockId::new(start),
        },
        block_span: span(start + 20),
        object: None,
        method: None,
        usage: ResolvedUsage::Explicit(value(start, text)),
    };
    let mut builder = OriginBuilder::new();
    let usage = super::super::usage::lower(
        &[contribution(70, "f()\n"), contribution(80, invalid)],
        &mut builder,
        &mut crate::diagnostic::Diagnostics::new(),
    )
    .expect("usage section");
    let usage_origin = span(90);
    builder.add_root(usage);
    let (document, map) = builder.materialize();
    let mut diagnostics = crate::diagnostic::Diagnostics::new();
    assert!(
        super::super::serialize::serialize(&document, &map, span(70), &mut diagnostics).is_none()
    );
    let primary = diagnostics
        .iter()
        .next()
        .expect("writer diagnostic")
        .primary
        .span;
    assert_eq!(primary, usage_origin, "diagnostics: {diagnostics:?}");
    assert_ne!(primary, span(70));
}

#[test]
fn identity_survives_inserting_a_sibling_before_a_node() {
    let mut builder = OriginBuilder::new();
    let held = builder.append_text("held");
    builder.record(held, &[span(20)]);
    builder.insert_root_before(held, RdNode::Text("\n".into()));
    let (document, map) = builder.materialize();
    assert_paths_address_nodes(&map, &document);
    let path = map
        .paths
        .iter()
        .find_map(|(path, id)| {
            (*id == held).then(|| {
                RdPath::new(
                    path.iter()
                        .enumerate()
                        .map(|(position, segment)| match segment {
                            OriginPathSegment::Child(index) if position == 0 => {
                                RdPathSegment::TopLevel(*index)
                            }
                            OriginPathSegment::Child(index) => RdPathSegment::Child(*index),
                            OriginPathSegment::Option => RdPathSegment::Option,
                        })
                        .collect(),
                )
            })
        })
        .expect("held node path");
    assert_eq!(span_for_path(&map, &path), Some(span(20)));
}

#[test]
fn external_content_uses_the_nearest_request_as_its_source_span() {
    let request = TagOrigin::Implicit {
        intro_span: span(42),
    };
    let content = ResolvedContent {
        value: InheritableContent::Rd(Vec::new()),
        provenance: InheritanceTrace {
            source: DocumentationOrigin::External {
                package: "pkg".to_owned(),
                topic: "topic".to_owned(),
                component: InheritField::Title,
            },
            requests: vec![request],
        },
    };
    assert_eq!(content_spans(&content), vec![span(42)]);
}
