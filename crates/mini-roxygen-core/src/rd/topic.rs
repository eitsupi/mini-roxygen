//! Topic preflight, section ordering, headers, and document assembly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rd_ast::{RdDocument, RdNode, RdTag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::generation::render_rd_header;
use crate::inherit::{InheritableContent, ResolvedContent, ResolvedRdTopic};
use crate::markdown_conversion::{LatexFragment, MarkdownContext};
use crate::model::TopicKey;
use crate::source::{SourceMap, Span, TextRange};

use super::arguments;
use super::origins::{LeafKind, NodeId, OriginBuilder, content_spans, tag_origin_spans};
use super::prose;
use super::sections;
use super::serialize;
use super::usage;

pub(crate) fn nice_name(name: &str) -> String {
    const SUBSTITUTIONS: &[(&str, &str)] = &[
        ("[<-", "-subset-"),
        ("[", "-sub-"),
        ("<-", "-set-"),
        ("::", "-"),
        ("!", "-not-"),
        ("&", "-and-"),
        ("|", "-or-"),
        ("*", "-times-"),
        ("+", "-plus-"),
        ("^", "-pow-"),
        ("\"", "-quote-"),
        ("#", "-hash-"),
        ("$", "-cash-"),
        ("%", "-grapes-"),
        ("'", "-single-quote-"),
        ("(", "-open-paren-"),
        (")", "-close-paren-"),
        (":", "-colon-"),
        (";", "-semi-colon-"),
        ("<", "-less-than-"),
        ("==", "-equals-"),
        ("=", "-equals-"),
        (">", "-greater-than-"),
        ("?", "-help-"),
        ("@", "-at-"),
        ("]", "-close-brace-"),
        ("\\", "-backslash-"),
        ("/", "-slash-"),
        ("`", "-tick-"),
        ("{", "-open-curly-"),
        ("}", "-close-"),
        ("~", "-twiddle-"),
    ];
    let mut value = name.to_owned();
    for (from, to) in SUBSTITUTIONS {
        value = value.replace(from, to);
    }
    // Every remaining run of unusable characters becomes one hyphen, and then
    // every run of hyphens collapses. The second step matters on its own: the
    // table above emits hyphens of its own, so `[[` leaves two adjacent ones
    // that no unusable character produced.
    let mut cleaned = String::new();
    for character in value.chars() {
        let character = if character.is_ascii_alphanumeric() || matches!(character, '_' | '.') {
            character
        } else {
            '-'
        };
        if character == '-' && cleaned.ends_with('-') {
            continue;
        }
        cleaned.push(character);
    }
    let cleaned = cleaned.trim_matches('-');
    match cleaned.strip_prefix('.') {
        Some(rest) => format!("dot-{rest}"),
        None => cleaned.to_owned(),
    }
}

/// Returns the topic key's output path, or `None` when normalization leaves no
/// name to build one from.
///
/// roxygen2 maps everything outside its allowed set to a separator and then
/// strips the separators, so a name written entirely in non-ASCII letters, or
/// entirely in operators the substitution table does not cover, normalizes away
/// to nothing. roxygen2 goes on to write that as `man/.Rd`: a hidden file that
/// every such topic silently overwrites. Refusing is the divergence worth
/// taking, since there is no correct roxygen2 filename left to match anyway.
pub(crate) fn output_path(key: &TopicKey) -> Option<PathBuf> {
    let name = nice_name(key.as_str());
    (!name.is_empty()).then(|| Path::new("man").join(format!("{name}.Rd")))
}

pub(crate) fn anchor(topic: &ResolvedRdTopic) -> Option<Span> {
    topic
        .title
        .as_ref()
        .and_then(|value| content_spans(value).first().copied())
        .or_else(|| topic.aliases.first().map(|alias| alias.span))
        .or_else(|| {
            topic
                .description
                .as_ref()
                .and_then(|value| content_spans(value).first().copied())
        })
        .or_else(|| {
            topic
                .note
                .as_ref()
                .and_then(|value| content_spans(value).first().copied())
        })
        .or_else(|| {
            topic
                .author
                .as_ref()
                .and_then(|value| content_spans(value).first().copied())
        })
        .or_else(|| {
            topic
                .blocks
                .first()
                .map(|block| Span::new(block.file, TextRange::new(0, 0)))
        })
}

pub(crate) fn build(
    key: &TopicKey,
    topic: &ResolvedRdTopic,
    sources: &SourceMap,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
) -> Option<(PathBuf, RdDocument, String)> {
    if topic.kind == crate::model::RdTopicKind::Data
        && topic.format.is_none()
        && !topic.kind_conflict_reported
        && let Some(span) = topic.missing_data_format_span
    {
        diagnostics.push(Diagnostic::new(
            Severity::Warning,
            DiagnosticCode::MissingDataFormat,
            format!(
                "data topic `{}` has no format; automatic data format generation is not available without evaluating R",
                key.as_str()
            ),
            Label::new(span, "data topic kind established here"),
        ));
    }
    if let Some(state) = &topic.package_metadata_diagnostics {
        if state.missing_description && topic.description.is_none() {
            diagnostics.push(Diagnostic::new(
                Severity::Warning,
                DiagnosticCode::MissingPackageDescription,
                format!("package topic `{}` has no description", key.as_str()),
                Label::new(
                    state.anchor,
                    "add Description to DESCRIPTION, add @description, or inherit a description",
                ),
            ));
        }
        if let Some(error) = &state.authors_parse_error
            && topic.author.is_none()
        {
            diagnostics.push(Diagnostic::new(
                Severity::Warning,
                DiagnosticCode::PackageAuthorsParse,
                format!("failed to parse Authors@R: {error}"),
                Label::new(state.anchor, "Authors@R could not be statically parsed"),
            ));
        }
    }
    let Some(title) = &topic.title else {
        let anchor = topic
            .package_metadata_diagnostics
            .as_ref()
            .map(|state| state.anchor)
            .or_else(|| anchor(topic))?;
        let (code, message, label) = if topic.kind == crate::model::RdTopicKind::Package {
            (
                DiagnosticCode::MissingPackageTitle,
                format!("package topic `{}` has no title", key.as_str()),
                "add Title to DESCRIPTION, add @title, or inherit a title",
            )
        } else {
            (
                DiagnosticCode::MissingTopicTitle,
                format!("topic `{}` has no title", key.as_str()),
                "a valid Rd topic requires a title",
            )
        };
        diagnostics.push(Diagnostic::new(
            Severity::Error,
            code,
            message,
            Label::new(anchor, label),
        ));
        return None;
    };
    let anchor = anchor(topic).or_else(|| content_spans(title).first().copied())?;
    let Some(path) = output_path(key) else {
        diagnostics.push(Diagnostic::new(
            Severity::Error,
            DiagnosticCode::UnnameableRdFile,
            format!(
                "topic `{}` has no Rd file name after normalization",
                key.as_str()
            ),
            Label::new(
                anchor,
                "a topic name must keep at least one usable character",
            ),
        ));
        return None;
    };
    let mut builder = OriginBuilder::new();
    builder.append_nodes(render_header(topic, sources));
    if matches!(
        topic.kind,
        crate::model::RdTopicKind::Package | crate::model::RdTopicKind::Data
    ) {
        let doc_type_text = builder.text_child(match topic.kind {
            crate::model::RdTopicKind::Package => "package",
            crate::model::RdTopicKind::Data => "data",
            crate::model::RdTopicKind::Ordinary => unreachable!(),
        });
        let doc_type = sections::plain(
            &mut builder,
            RdTag::DocType,
            vec![doc_type_text],
            LeafKind::Text,
        );
        add_section(&mut builder, doc_type, false);
    }
    let diagnostics_before = diagnostics.len();

    let name_section = sections::verbatim(&mut builder, RdTag::Name, topic.name.0.clone());
    add_section(&mut builder, name_section, false);
    let mut seen_aliases = BTreeSet::new();
    for alias in &topic.aliases {
        if !seen_aliases.insert(alias.name.0.clone()) {
            continue;
        }
        let alias_leaf = builder.verb_child(alias.name.0.clone());
        builder.record(
            alias_leaf,
            &tag_origin_spans(&crate::tags::TagOrigin::Implicit {
                intro_span: alias.span,
            }),
        );
        let alias_section =
            sections::plain(&mut builder, RdTag::Alias, vec![alias_leaf], LeafKind::Verb);
        add_section(&mut builder, alias_section, false);
    }
    let title_fragment = resolved_fragment(title, context, diagnostics);
    add_plain_fragment_section(&mut builder, RdTag::Title, title, &title_fragment);

    for (tag, value) in [
        (RdTag::Format, topic.format.as_ref()),
        (RdTag::Source, topic.source.as_ref()),
    ] {
        if let Some(value) = value {
            add_resolved_section(&mut builder, tag, value, context, diagnostics);
        }
    }
    if let Some(usage_node) = usage::lower(&topic.usages, &mut builder, diagnostics) {
        add_section(&mut builder, usage_node, false);
    }
    if !topic.params.is_empty() {
        let node = arguments::lower(&topic.params, context, &mut builder, diagnostics);
        add_section(&mut builder, node, false);
    }
    if let Some(value) = &topic.return_value {
        add_resolved_section(&mut builder, RdTag::Value, value, context, diagnostics);
    }
    if let Some(value) = &topic.description {
        add_resolved_section(
            &mut builder,
            RdTag::Description,
            value,
            context,
            diagnostics,
        );
    } else if !topic.description_suppressed {
        add_spaced_fragment_section(&mut builder, RdTag::Description, title, &title_fragment);
    }
    if let Some(value) = &topic.details {
        add_resolved_section(&mut builder, RdTag::Details, value, context, diagnostics);
    }
    if let Some(value) = &topic.note {
        add_resolved_section(&mut builder, RdTag::Note, value, context, diagnostics);
    }
    for section in &topic.sections {
        let title_nodes =
            append_resolved_content(&section.title, context, diagnostics, &mut builder);
        let body_nodes = append_resolved_content(&section.body, context, diagnostics, &mut builder);
        let section_node = sections::named_section(&mut builder, title_nodes, body_nodes);
        builder.record(
            section_node,
            &content_spans(&section.title)
                .into_iter()
                .chain(content_spans(&section.body))
                .collect::<Vec<_>>(),
        );
        add_section(&mut builder, section_node, true);
    }
    if let Some(value) = &topic.examples {
        let example_nodes =
            append_spaced_resolved_content(value, context, diagnostics, &mut builder);
        let example_spans = content_spans(value);
        let examples = sections::spaced(
            &mut builder,
            RdTag::Examples,
            example_nodes,
            LeafKind::RCode,
        );
        builder.record(examples, &example_spans);
        add_section(&mut builder, examples, false);
    }
    for (tag, value) in [
        (RdTag::References, topic.references.as_ref()),
        (RdTag::SeeAlso, topic.see_also.as_ref()),
        (RdTag::Author, topic.author.as_ref()),
    ] {
        if let Some(value) = value {
            add_resolved_section(&mut builder, tag, value, context, diagnostics);
        } else if tag == RdTag::SeeAlso {
            if let Some(value) = &topic.package_see_also {
                add_package_seealso(&mut builder, value);
            }
        } else if tag == RdTag::Author
            && let Some(value) = &topic.package_author
        {
            add_package_author(&mut builder, value);
        }
    }
    let mut keywords = topic
        .keywords
        .iter()
        .map(|keyword| keyword.0.clone())
        .collect::<Vec<_>>();
    keywords.sort();
    keywords.dedup();
    for keyword in keywords {
        let keyword = builder.text_child(keyword);
        let section = sections::plain(&mut builder, RdTag::Keyword, vec![keyword], LeafKind::Text);
        add_section(&mut builder, section, false);
    }

    let (document, origins) = builder.materialize();
    #[cfg(test)]
    super::origins::assert_paths_address_nodes(&origins, &document);
    if diagnostics
        .iter()
        .skip(diagnostics_before)
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return None;
    }
    let content = serialize::serialize(&document, &origins, anchor, diagnostics)?;
    Some((path, document, content))
}

fn add_package_author(builder: &mut OriginBuilder, value: &crate::model::PackageAuthor) {
    let mut children = Vec::new();
    if !value.maintainers.is_empty() {
        let strong_text = builder.text_child("Maintainer");
        children.push(builder.tagged_child(RdTag::Strong, None, vec![strong_text]));
        append_text(builder, &mut children, ": ");
        append_people(builder, &mut children, &value.maintainers);
        sections::append_newlines(builder, &mut children, LeafKind::Text, 2);
    }
    append_people_section(builder, &mut children, "Authors:", &value.authors);
    append_people_section(
        builder,
        &mut children,
        "Other contributors:",
        &value.other_contributors,
    );
    let section = sections::spaced(builder, RdTag::Author, children, LeafKind::Text);
    add_section(builder, section, false);
}

fn append_people_section(
    builder: &mut OriginBuilder,
    out: &mut Vec<NodeId>,
    label: &str,
    people: &[crate::model::PackagePerson],
) {
    if people.is_empty() {
        return;
    }
    sections::append_newlines(builder, out, LeafKind::Text, 2);
    append_text(builder, out, &format!("{label}\n"));
    out.push(build_people_itemize(builder, people));
}

fn build_people_itemize(
    builder: &mut OriginBuilder,
    people: &[crate::model::PackagePerson],
) -> NodeId {
    let mut items = Vec::new();
    for person in people {
        items.push(builder.tagged_child(RdTag::Item, None, vec![]));
        append_text(builder, &mut items, " ");
        append_person(builder, &mut items, person);
        sections::append_newlines(builder, &mut items, LeafKind::Text, 1);
    }
    builder.tagged_child(RdTag::Itemize, None, items)
}

fn append_people(
    builder: &mut OriginBuilder,
    out: &mut Vec<NodeId>,
    people: &[crate::model::PackagePerson],
) {
    for (index, person) in people.iter().enumerate() {
        if index > 0 {
            append_text(builder, out, ", ");
        }
        append_person(builder, out, person);
    }
}

fn append_text(builder: &mut OriginBuilder, out: &mut Vec<NodeId>, value: &str) {
    if let Some(last) = out.last().copied()
        && builder.leaf_matches(last, LeafKind::Text)
        && !builder.leaf_ends_with_newline(last)
        && !value.contains('\n')
    {
        builder.extend_leaf(last, value);
    } else {
        out.push(builder.text_child(value));
    }
}

fn append_person(
    builder: &mut OriginBuilder,
    out: &mut Vec<NodeId>,
    person: &crate::model::PackagePerson,
) {
    append_text(builder, out, &person.name);
    if let Some(email) = &person.email {
        append_text(builder, out, " ");
        let email_node = builder.text_child(email.clone());
        out.push(builder.tagged_child(RdTag::Email, None, vec![email_node]));
    }
    for identity in &person.identities {
        append_text(builder, out, " (");
        let url_text = builder.verb_child(identity.href.clone());
        let url = builder.group_child(vec![url_text]);
        let label_text = builder.text_child(identity.label.clone());
        let label = builder.group_child(vec![label_text]);
        out.push(builder.tagged_child(RdTag::Href, None, vec![url, label]));
        append_text(builder, out, ")");
    }
    if !person.comments.is_empty() {
        append_text(builder, out, " (");
        for (index, comment) in person.comments.iter().enumerate() {
            if index > 0 {
                append_text(builder, out, ", ");
            }
            if let Some(label) = &comment.label {
                append_text(builder, out, label);
                append_text(builder, out, ": ");
            }
            append_text(builder, out, &comment.value);
        }
        append_text(builder, out, ")");
    }
    if !person.roles.is_empty() {
        append_text(builder, out, " [");
        append_text(builder, out, &person.roles.join(", "));
        append_text(builder, out, "]");
    }
}

fn add_package_seealso(builder: &mut OriginBuilder, value: &crate::model::PackageSeeAlso) {
    let mut items = Vec::new();
    for link in &value.urls {
        let body = if link.doi {
            let text = builder.text_child(link.target.clone());
            builder.tagged_child(RdTag::Doi, None, vec![text])
        } else {
            let text = builder.verb_child(link.target.clone());
            builder.tagged_child(RdTag::Url, None, vec![text])
        };
        items.push(builder.tagged_child(RdTag::Item, None, vec![]));
        items.push(body);
        items.push(builder.text_child("\n"));
    }
    if let Some(bugs) = &value.bug_reports {
        items.push(builder.tagged_child(RdTag::Item, None, vec![]));
        items.push(builder.text_child(" Report bugs at "));
        let bug_text = builder.verb_child(bugs.clone());
        items.push(builder.tagged_child(RdTag::Url, None, vec![bug_text]));
        items.push(builder.text_child("\n"));
    }
    let mut children = vec![builder.text_child("Useful links:\n")];
    children.push(builder.tagged_child(RdTag::Itemize, None, items));
    let section = sections::spaced(builder, RdTag::SeeAlso, children, LeafKind::Text);
    add_section(builder, section, false);
}

fn render_header(topic: &ResolvedRdTopic, sources: &SourceMap) -> Vec<RdNode> {
    let paths = topic
        .blocks
        .iter()
        .filter_map(|block| sources.get(block.file).map(|file| file.path()))
        .collect::<Vec<_>>();
    render_rd_header(paths)
}

fn add_resolved_section(
    builder: &mut OriginBuilder,
    tag: RdTag,
    content: &ResolvedContent,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
) {
    let fragment = resolved_fragment(content, context, diagnostics);
    add_spaced_fragment_section(builder, tag, content, &fragment);
}

fn add_spaced_fragment_section(
    builder: &mut OriginBuilder,
    tag: RdTag,
    content: &ResolvedContent,
    fragment: &LatexFragment,
) {
    let mut children = prose::append_fragment(builder, fragment);
    trim_inherited_rd_nodes(builder, content, &mut children);
    let spans = content_spans(content);
    let section = sections::spaced(builder, tag, children, LeafKind::Text);
    builder.record(section, &spans);
    add_section(builder, section, false);
}

fn trim_inherited_rd_nodes(
    builder: &mut OriginBuilder,
    content: &ResolvedContent,
    nodes: &mut Vec<NodeId>,
) {
    if !matches!(content.value, InheritableContent::Rd(_)) {
        return;
    }

    // roxygen2 trims an external field while extracting it. The external
    // provider is not implemented yet, so apply that rule at this Rd
    // boundary; the provider must not trim the same value a second time.
    for kind in [LeafKind::Text, LeafKind::RCode, LeafKind::Verb] {
        sections::trim_top_level(builder, nodes, kind);
    }
}

fn append_spaced_resolved_content(
    content: &ResolvedContent,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
    builder: &mut OriginBuilder,
) -> Vec<NodeId> {
    let mut nodes = append_resolved_content(content, context, diagnostics, builder);
    trim_inherited_rd_nodes(builder, content, &mut nodes);
    nodes
}

fn add_plain_fragment_section(
    builder: &mut OriginBuilder,
    tag: RdTag,
    content: &ResolvedContent,
    fragment: &LatexFragment,
) {
    let children = prose::append_fragment(builder, fragment);
    let spans = content_spans(content);
    let section = sections::plain(builder, tag, children, LeafKind::Text);
    builder.record(section, &spans);
    add_section(builder, section, false);
}

fn append_resolved_content(
    content: &ResolvedContent,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
    builder: &mut OriginBuilder,
) -> Vec<NodeId> {
    let fragment = resolved_fragment(content, context, diagnostics);
    let nodes = prose::append_fragment(builder, &fragment);
    let spans = content_spans(content);
    for node in &nodes {
        builder.record(*node, &spans);
    }
    nodes
}

fn resolved_fragment(
    content: &ResolvedContent,
    context: &MarkdownContext<'_>,
    diagnostics: &mut Diagnostics,
) -> LatexFragment {
    match &content.value {
        InheritableContent::Markdown(value) => prose::convert(value, context, diagnostics),
        InheritableContent::RCode(value) => prose::rcode_fragment(value.as_str()),
        InheritableContent::Examples(value) => {
            let fallback = content_spans(content).first().copied();
            prose::rd_fragment(examples_nodes(value, diagnostics, fallback))
        }
        InheritableContent::Rd(nodes) => prose::rd_fragment(nodes.clone()),
    }
}

fn examples_nodes(
    value: &crate::tags::ExamplesContent,
    diagnostics: &mut Diagnostics,
    fallback: Option<Span>,
) -> Vec<RdNode> {
    match value {
        crate::tags::ExamplesContent::Ordinary(value) => {
            super::examples_raw_rd::lower(value, diagnostics, fallback)
        }
        crate::tags::ExamplesContent::Conditional(value) => {
            let prefix = RdNode::tagged(
                RdTag::DontShow,
                None,
                vec![
                    RdNode::RCode("if ({\n".to_owned()),
                    RdNode::RCode(format!("{}\n", value.condition.as_str())),
                    RdNode::RCode("}) withAutoprint({ # examplesIf".to_owned()),
                ],
            );
            let close = RdNode::tagged(
                RdTag::DontShow,
                None,
                vec![RdNode::RCode("}) # examplesIf".to_owned())],
            );
            let mut nodes = vec![prefix, RdNode::RCode("\n".to_owned())];
            nodes.extend(super::examples_raw_rd::lower(
                &value.body,
                diagnostics,
                fallback,
            ));
            match nodes.last_mut() {
                Some(RdNode::RCode(value)) if !value.ends_with('\n') => value.push('\n'),
                _ => nodes.push(RdNode::RCode("\n".to_owned())),
            }
            nodes.push(close);
            nodes
        }
    }
}

fn add_section(builder: &mut OriginBuilder, node: NodeId, named: bool) {
    builder.add_root(node);
    let separator_count = if named { 2 } else { 1 };
    for _ in 0..separator_count {
        builder.append_text("\n");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rd_ast::{RdDocument, RdNode, RdTag};
    use rd_writer::{Writer, WriterOptions};

    use crate::diagnostic::Diagnostics;
    use crate::inherit::{
        DocumentationError, DocumentationOrigin, DocumentationProvider, ExternalInheritancePolicy,
        ExternalPolicySource, InheritableContent, InheritableTopic, InheritanceOptions,
        InheritanceTrace, ResolvedContent, TopicExistence, TopicRequest, project_external_topic,
    };
    use crate::model::TopicKey;
    use crate::namespace::EmptyS3GenericProvider;
    use crate::package::{PackageInputs, PackageMetadata};
    use crate::pipeline::{DocumentOptions, document_package_with_options_and_providers};
    use crate::source::{FileId, SourceFile, SourceMap, Span, TextRange};
    use crate::tags::{MarkdownText, NormalizeHead, SourcedText, TagOrigin};

    use super::super::origins::LeafKind;
    use super::super::origins::OriginBuilder;
    use super::super::sections;
    use super::{add_section, append_resolved_content, nice_name};

    fn inherited_rd(nodes: Vec<RdNode>) -> ResolvedContent {
        ResolvedContent {
            value: InheritableContent::Rd(nodes),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::External {
                    package: "donor".to_owned(),
                    topic: "topic".to_owned(),
                    component: crate::tags::InheritField::Description,
                },
                requests: Vec::new(),
            },
        }
    }

    fn write(document: &rd_ast::RdDocument) -> String {
        Writer::new(WriterOptions::default())
            .write_document(document)
            .expect("topic is writer-valid")
    }

    struct Links;

    impl crate::markdown_conversion::HelpLinkResolver for Links {
        fn resolve_unqualified(&self, _topic: &str) -> crate::markdown_conversion::LinkResolution {
            crate::markdown_conversion::LinkResolution::Local
        }
    }

    struct ExternalLinkProvider;

    impl DocumentationProvider for ExternalLinkProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { package, topic } = request else {
                return Ok(None);
            };
            if package != "donor" || topic.0 != "donor_topic" {
                return Ok(None);
            }
            let document = RdDocument::new(vec![RdNode::tagged(
                RdTag::Arguments,
                None,
                vec![RdNode::tagged(
                    RdTag::Item,
                    None,
                    vec![
                        RdNode::group(vec![RdNode::Text("call".into())]),
                        RdNode::group(vec![
                            RdNode::Text("See the ".into()),
                            RdNode::tagged(RdTag::Code, None, vec![RdNode::RCode("call".into())]),
                            RdNode::Text(" argument of ".into()),
                            RdNode::tagged(
                                RdTag::Code,
                                None,
                                vec![RdNode::tagged(
                                    RdTag::Link,
                                    Some(vec![RdNode::Text("=abort".into())]),
                                    vec![RdNode::Text("abort()".into())],
                                )],
                            ),
                            RdNode::Text(" for more information.".into()),
                        ]),
                    ],
                )],
            )]);
            Ok(Some(project_external_topic(
                package,
                "donor_topic",
                &document,
                self,
            )))
        }

        fn topic_exists(&self, package: &str, alias: &str) -> TopicExistence {
            TopicExistence::Known(package == "donor" && alias == "abort")
        }
    }

    struct ParameterLabelProvider;

    impl DocumentationProvider for ParameterLabelProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { package, topic } = request else {
                return Ok(None);
            };
            if package != "donor" || topic.0 != "parameter_topic" {
                return Ok(None);
            }
            let link = RdNode::tagged(
                RdTag::Link,
                Some(vec![RdNode::Text("=z".into())]),
                vec![RdNode::Text("z".into())],
            );
            let document = RdDocument::new(vec![RdNode::tagged(
                RdTag::Arguments,
                None,
                vec![RdNode::tagged(
                    RdTag::Item,
                    None,
                    vec![
                        RdNode::group(vec![
                            RdNode::tagged(RdTag::Code, None, vec![RdNode::RCode("x".into())]),
                            RdNode::Text(", ".into()),
                            link,
                            RdNode::Text(", ".into()),
                            RdNode::tagged(RdTag::Dots, None, Vec::new()),
                        ]),
                        RdNode::group(vec![RdNode::Text("Description".into())]),
                    ],
                )],
            )]);
            Ok(Some(project_external_topic(
                package,
                "parameter_topic",
                &document,
                self,
            )))
        }

        fn topic_exists(&self, package: &str, alias: &str) -> TopicExistence {
            TopicExistence::Known(package == "donor" && alias == "z")
        }
    }

    #[test]
    fn external_inheritance_absolutizes_links_in_full_rd_output() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/target.R"),
            r#"#' @name target
#' @title Target
#' @inheritParams donor::donor_topic call
target <- function(call = NULL) NULL
"#
            .to_owned(),
        ));
        let inputs = PackageInputs {
            sources,
            metadata: PackageMetadata::new("recipient", None).unwrap(),
        };
        let options = DocumentOptions {
            inline_r_substitutions: crate::inline_r::InlineRSubstitutions::builtins().unwrap(),
            s3_registrars: Default::default(),
        };
        let inheritance_options = InheritanceOptions {
            external: ExternalInheritancePolicy::Strict,
            external_source: ExternalPolicySource::Explicit,
        };
        let output = document_package_with_options_and_providers(
            &inputs,
            &options,
            &EmptyS3GenericProvider,
            &ExternalLinkProvider,
            &inheritance_options,
        );
        assert!(
            !output.has_errors(),
            "{:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
        let generated = output
            .rd
            .files
            .get(&TopicKey("target".into()))
            .expect("inherited target Rd");

        insta::assert_snapshot!(generated.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/target.R
\name{target}
\alias{target}
\title{Target}
\usage{
target(call = NULL)
}
\arguments{
\item{call}{See the \code{call} argument of \code{\link[donor:abort]{abort()}} for more information.}
}
\description{
Target
}
        "###);
    }

    #[test]
    fn external_parameter_labels_survive_full_resolution_and_rd_build() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/parameter-label.R"),
            r#"#' @name target
#' @title Target
#' @inheritParams donor::parameter_topic
target <- function(x, z, ...) NULL
"#
            .to_owned(),
        ));
        let inputs = PackageInputs {
            sources,
            metadata: PackageMetadata::new("recipient", None).unwrap(),
        };
        let options = DocumentOptions {
            inline_r_substitutions: crate::inline_r::InlineRSubstitutions::builtins().unwrap(),
            s3_registrars: Default::default(),
        };
        let inheritance_options = InheritanceOptions {
            external: ExternalInheritancePolicy::Strict,
            external_source: ExternalPolicySource::Explicit,
        };
        let output = document_package_with_options_and_providers(
            &inputs,
            &options,
            &EmptyS3GenericProvider,
            &ParameterLabelProvider,
            &inheritance_options,
        );
        assert!(
            !output.has_errors(),
            "{:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
        let generated = output
            .rd
            .files
            .get(&TopicKey("target".into()))
            .expect("inherited target Rd");
        insta::assert_snapshot!(generated.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/parameter-label.R
\name{target}
\alias{target}
\title{Target}
\usage{
target(x, z, ...)
}
\arguments{
\item{\code{x}, \link[donor:z]{z}, \dots}{Description}
}
\description{
Target
}
"###);
        crate::rd_oracle::assert_r_accepts(&generated.content);
    }

    #[test]
    fn formal_reorder_falls_back_to_plain_parameter_names_in_full_rd_output() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/parameter-reorder.R"),
            r#"#' @name target
#' @title Target
#' @inheritParams donor::parameter_topic
target <- function(z, x, ...) NULL
"#
            .to_owned(),
        ));
        let inputs = PackageInputs {
            sources,
            metadata: PackageMetadata::new("recipient", None).unwrap(),
        };
        let options = DocumentOptions {
            inline_r_substitutions: crate::inline_r::InlineRSubstitutions::builtins().unwrap(),
            s3_registrars: Default::default(),
        };
        let inheritance_options = InheritanceOptions {
            external: ExternalInheritancePolicy::Strict,
            external_source: ExternalPolicySource::Explicit,
        };
        let output = document_package_with_options_and_providers(
            &inputs,
            &options,
            &EmptyS3GenericProvider,
            &ParameterLabelProvider,
            &inheritance_options,
        );
        assert!(
            !output.has_errors(),
            "{:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
        let generated = output
            .rd
            .files
            .get(&TopicKey("target".into()))
            .expect("inherited target Rd");
        insta::assert_snapshot!(generated.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/parameter-reorder.R
\name{target}
\alias{target}
\title{Target}
\usage{
target(z, x, ...)
}
\arguments{
\item{z, x, ...}{Description}
}
\description{
Target
}
"###);
        crate::rd_oracle::assert_r_accepts(&generated.content);
    }

    #[test]
    fn nice_name_applies_ordered_substitutions_and_dot_cleanup() {
        assert_eq!(nice_name("[<-"), "subset");
        assert_eq!(nice_name(".hidden"), "dot-hidden");
        assert_eq!(nice_name("a==b"), "a-equals-b");
        assert_eq!(nice_name("[[.myclass"), "sub-sub-.myclass");
        assert_eq!(nice_name("%in%"), "grapes-in-grapes");
        assert_eq!(nice_name("[.data.frame"), "sub-.data.frame");
        assert_eq!(nice_name(".onLoad"), "dot-onLoad");
        assert_eq!(nice_name("x<-"), "x-set");
        assert_eq!(nice_name("a b"), "a-b");
        assert_eq!(nice_name("foo"), "foo");
    }

    #[test]
    fn examples_content_keeps_an_inner_origin() {
        let source = SourceFile::new(PathBuf::from("test.R"), "`x` bad".into());
        let content = ResolvedContent {
            value: InheritableContent::Markdown(MarkdownText::new(SourcedText::from_lines(
                &source,
                &[Span::new(FileId::new(0), TextRange::new(0, 7))],
                NormalizeHead::Intro,
            ))),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::Local(TagOrigin::Implicit {
                    intro_span: Span::new(FileId::new(0), TextRange::new(20, 22)),
                }),
                requests: Vec::new(),
            },
        };
        let context = crate::markdown_conversion::MarkdownContext {
            current_package: None,
            links: &Links,
            inline_r_session: None,
        };
        let mut builder = OriginBuilder::new();
        let nodes =
            append_resolved_content(&content, &context, &mut Diagnostics::new(), &mut builder);
        let examples = sections::spaced(&mut builder, RdTag::Examples, nodes, LeafKind::RCode);
        builder.add_root(examples);
        let (document, origins) = builder.materialize();
        let mut diagnostics = Diagnostics::new();
        assert!(
            super::super::serialize::serialize(
                &document,
                &origins,
                Span::new(FileId::new(0), TextRange::new(0, 1)),
                &mut diagnostics,
            )
            .is_none()
        );
        assert_eq!(
            diagnostics
                .iter()
                .next()
                .expect("examples writer diagnostic")
                .primary
                .span,
            Span::new(FileId::new(0), TextRange::new(3, 7)),
        );
    }

    #[test]
    fn inherited_rd_single_field_trims_wrapper_whitespace() {
        let content = inherited_rd(vec![RdNode::Text("\nBody\n".to_owned())]);
        let mut builder = OriginBuilder::new();
        let context = crate::markdown_conversion::MarkdownContext {
            current_package: None,
            links: &Links,
            inline_r_session: None,
        };
        let mut diagnostics = Diagnostics::new();
        super::add_resolved_section(
            &mut builder,
            RdTag::Description,
            &content,
            &context,
            &mut diagnostics,
        );
        let (document, _) = builder.materialize();

        assert_eq!(write(&document), "\\description{\nBody\n}\n");
    }

    #[test]
    fn inherited_rd_examples_trim_wrapper_whitespace() {
        let content = inherited_rd(vec![RdNode::RCode("\nexample()\n".to_owned())]);
        let mut builder = OriginBuilder::new();
        let context = crate::markdown_conversion::MarkdownContext {
            current_package: None,
            links: &Links,
            inline_r_session: None,
        };
        let mut diagnostics = Diagnostics::new();
        let nodes = super::append_spaced_resolved_content(
            &content,
            &context,
            &mut diagnostics,
            &mut builder,
        );
        let examples = sections::spaced(&mut builder, RdTag::Examples, nodes, LeafKind::RCode);
        add_section(&mut builder, examples, false);
        let (document, _) = builder.materialize();

        assert_eq!(write(&document), "\\examples{\nexample()\n}\n");
    }

    #[test]
    fn inherited_rd_trims_only_outer_leaves_and_local_section_bytes_remain() {
        let inherited = inherited_rd(vec![
            RdNode::Text("\nBody  ".to_owned()),
            RdNode::tagged(
                RdTag::Emph,
                None,
                vec![RdNode::Text("  nested  ".to_owned())],
            ),
            RdNode::Text("  tail\n".to_owned()),
        ]);
        let mut builder = OriginBuilder::new();
        let context = crate::markdown_conversion::MarkdownContext {
            current_package: None,
            links: &Links,
            inline_r_session: None,
        };
        let mut diagnostics = Diagnostics::new();
        super::add_resolved_section(
            &mut builder,
            RdTag::Description,
            &inherited,
            &context,
            &mut diagnostics,
        );
        let (document, _) = builder.materialize();
        assert_eq!(
            write(&document),
            "\\description{\nBody  \\emph{  nested  }  tail\n}\n"
        );

        let local = ResolvedContent {
            value: InheritableContent::Markdown(crate::markdown_conversion::test_support::value(
                "\nBody\n",
            )),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::Local(TagOrigin::Implicit {
                    intro_span: Span::new(FileId::new(0), TextRange::new(0, 1)),
                }),
                requests: Vec::new(),
            },
        };
        let mut builder = OriginBuilder::new();
        let mut diagnostics = Diagnostics::new();
        let title = builder.text_child("Title");
        let body = append_resolved_content(&local, &context, &mut diagnostics, &mut builder);
        let section = sections::named_section(&mut builder, vec![title], body);
        add_section(&mut builder, section, true);
        let (document, _) = builder.materialize();
        assert_eq!(write(&document), "\\section{Title}{\n Body\n}\n\n");
    }

    #[test]
    fn named_sections_have_a_blank_following_separator_and_two_at_eof() {
        let mut builder = OriginBuilder::new();
        let title = builder.text_child("Title");
        let body = builder.text_child("Body");
        let section = sections::named_section(&mut builder, vec![title], vec![body]);
        add_section(&mut builder, section, true);
        let description = builder.text_child("Description");
        let description = sections::plain(
            &mut builder,
            RdTag::Description,
            vec![description],
            LeafKind::Text,
        );
        add_section(&mut builder, description, false);
        let (document, _) = builder.materialize();
        assert_eq!(
            Writer::new(WriterOptions::default())
                .write_document(&document)
                .expect("topic is writer-valid"),
            "\\section{Title}{\n Body\n}\n\n\\description{Description}\n"
        );

        let mut builder = OriginBuilder::new();
        let title = builder.text_child("Title");
        let body = builder.text_child("Body");
        let section = sections::named_section(&mut builder, vec![title], vec![body]);
        add_section(&mut builder, section, true);
        let (document, _) = builder.materialize();
        assert_eq!(
            Writer::new(WriterOptions::default())
                .write_document(&document)
                .expect("topic is writer-valid"),
            "\\section{Title}{\n Body\n}\n\n"
        );
    }
}
