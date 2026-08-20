//! Projection of lowered external Rd documents into the inheritance boundary.

use rd_ast::{RdDocument, RdNode, RdPath, RdTag, RdTextSymbolKind, text_contents};

use crate::tags::{InheritField, ParamName};

use super::provider::{DocumentationIdentity, DocumentationProvider, TopicExistence};
use super::types::{
    DocumentationOrigin, InheritableContent, InheritableFields, InheritableParamGroup,
    InheritableParamLabel, InheritableSection, InheritableTopic, InheritanceTrace, ResolvedContent,
};

/// Projects a lowered external Rd document without depending on its storage.
///
/// The topic must be the canonical key returned by the help database after
/// alias resolution. Donor-relative links are qualified while their package
/// context is still known; all other Rd nodes retain their original shape.
#[must_use]
pub fn project_external_topic(
    package: &str,
    canonical_topic: &str,
    document: &RdDocument,
    provider: &dyn DocumentationProvider,
) -> InheritableTopic {
    let source = DocumentationIdentity::External {
        package: package.to_owned(),
        topic: canonical_topic.to_owned(),
    };

    InheritableTopic {
        identity: source,
        params: document
            .arguments()
            .map(|argument| {
                let label_nodes = argument.name.to_vec();
                InheritableParamGroup {
                    names: parameter_names(argument.name),
                    label: parameter_label(&label_nodes, package, provider),
                    description: external_content(
                        argument.description,
                        package,
                        canonical_topic,
                        InheritField::Params,
                        provider,
                    ),
                }
            })
            .collect(),
        fields: InheritableFields {
            title: document.title().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Title,
                    provider,
                )
            }),
            description: document.description().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Description,
                    provider,
                )
            }),
            details: document.details().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Details,
                    provider,
                )
            }),
            return_value: document.value().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Return,
                    provider,
                )
            }),
            see_also: document.see_also().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::SeeAlso,
                    provider,
                )
            }),
            references: document.references().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::References,
                    provider,
                )
            }),
            examples: document.examples().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Examples,
                    provider,
                )
            }),
            author: document.author().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Author,
                    provider,
                )
            }),
            source: document.source().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Source,
                    provider,
                )
            }),
            note: document.note().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Note,
                    provider,
                )
            }),
            format: document.format().map(|nodes| {
                external_content(
                    nodes,
                    package,
                    canonical_topic,
                    InheritField::Format,
                    provider,
                )
            }),
        },
        sections: document
            .sections()
            .map(|section| InheritableSection {
                title: external_content(
                    section.title,
                    package,
                    canonical_topic,
                    InheritField::Sections,
                    provider,
                ),
                body: external_content(
                    section.body,
                    package,
                    canonical_topic,
                    InheritField::Sections,
                    provider,
                ),
            })
            .collect(),
        requests: Vec::new(),
    }
}

fn parameter_names(nodes: &[RdNode]) -> Vec<ParamName> {
    split_parameter_label(nodes)
        .into_iter()
        .map(|fragment| text_contents_with_zero_arg_dots(&fragment))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .map(|name| ParamName(name.to_owned()))
        .collect()
}

fn parameter_label(
    nodes: &[RdNode],
    package: &str,
    provider: &dyn DocumentationProvider,
) -> InheritableParamLabel {
    let fragments = split_parameter_label(nodes);
    let has_empty_fragment = fragments
        .iter()
        .any(|fragment| text_contents_with_zero_arg_dots(fragment).trim().is_empty());
    if has_empty_fragment {
        InheritableParamLabel::Generated
    } else {
        InheritableParamLabel::Rd(absolutize_external_links(nodes.to_vec(), package, provider))
    }
}

fn split_parameter_label(nodes: &[RdNode]) -> Vec<Vec<RdNode>> {
    let mut fragments = vec![Vec::new()];
    for node in nodes {
        match node {
            RdNode::Text(text) => {
                let mut start = 0;
                for (index, character) in text.char_indices() {
                    if character != ',' {
                        continue;
                    }
                    if start < index {
                        fragments
                            .last_mut()
                            .expect("parameter fragment")
                            .push(RdNode::Text(text[start..index].to_owned()));
                    }
                    fragments.push(Vec::new());
                    start = index + character.len_utf8();
                }
                if start < text.len() {
                    fragments
                        .last_mut()
                        .expect("parameter fragment")
                        .push(RdNode::Text(text[start..].to_owned()));
                }
            }
            node => fragments
                .last_mut()
                .expect("parameter fragment")
                .push(node.clone()),
        }
    }
    fragments
}

fn text_contents_with_zero_arg_dots(nodes: &[RdNode]) -> String {
    let mut output = String::new();
    append_text_contents_with_zero_arg_dots(nodes, &RdPath::new(Vec::new()), &mut output);
    output
}

fn append_text_contents_with_zero_arg_dots(
    nodes: &[RdNode],
    parent_path: &RdPath,
    output: &mut String,
) {
    for (index, node) in nodes.iter().enumerate() {
        let path = parent_path.with_child(index);
        if let Some(symbol) = node.text_symbol(&path)
            && matches!(
                symbol.kind(),
                RdTextSymbolKind::Dots | RdTextSymbolKind::LDots
            )
        {
            output.push_str(symbol.fallback_text());
            continue;
        }
        match node {
            RdNode::Tagged(tagged) => {
                append_text_contents_with_zero_arg_dots(tagged.children(), &path, output);
            }
            RdNode::Group(group) => {
                append_text_contents_with_zero_arg_dots(group.children(), &path, output);
            }
            RdNode::Raw(raw) => {
                append_text_contents_with_zero_arg_dots(raw.children(), &path, output);
            }
            _ => output.push_str(&text_contents(std::slice::from_ref(node))),
        }
    }
}

fn external_content(
    nodes: &[RdNode],
    package: &str,
    canonical_topic: &str,
    component: InheritField,
    provider: &dyn DocumentationProvider,
) -> ResolvedContent {
    ResolvedContent {
        value: InheritableContent::Rd(absolutize_external_links(nodes.to_vec(), package, provider)),
        provenance: InheritanceTrace {
            source: DocumentationOrigin::External {
                package: package.to_owned(),
                topic: canonical_topic.to_owned(),
                component,
            },
            requests: Vec::new(),
        },
    }
}

fn absolutize_external_links(
    nodes: Vec<RdNode>,
    package: &str,
    provider: &dyn DocumentationProvider,
) -> Vec<RdNode> {
    nodes
        .into_iter()
        .map(|node| absolutize_external_link(node, package, provider))
        .collect()
}

fn absolutize_external_link(
    node: RdNode,
    package: &str,
    provider: &dyn DocumentationProvider,
) -> RdNode {
    let replacement = external_link_replacement(&node, package, provider);
    match node {
        RdNode::Tagged(tagged) => {
            let (mut tag, option, children) = tagged.into_parts();
            let mut option =
                option.map(|nodes| absolutize_external_links(nodes, package, provider));
            let children = absolutize_external_links(children, package, provider);
            if let Some((replacement_tag, replacement_option)) = replacement {
                tag = replacement_tag;
                option = Some(vec![RdNode::Text(replacement_option)]);
            }
            RdNode::tagged(tag, option, children)
        }
        RdNode::Group(group) => RdNode::group(absolutize_external_links(
            group.into_children(),
            package,
            provider,
        )),
        RdNode::Raw(raw) => {
            let (tag, option, children, payload, attributes) = raw.into_parts();
            RdNode::Raw(rd_ast::producer::raw_node(
                tag,
                option.map(|nodes| absolutize_external_links(nodes, package, provider)),
                absolutize_external_links(children, package, provider),
                payload,
                attributes,
            ))
        }
        leaf => leaf,
    }
}

fn external_link_replacement(
    node: &RdNode,
    package: &str,
    provider: &dyn DocumentationProvider,
) -> Option<(RdTag, String)> {
    let path = RdPath::new(Vec::new());
    if let Some(tagged) = node.as_tagged()
        && tagged.tag() == &RdTag::Link
    {
        let link = tagged.inspect_link(&path).ok()?;
        let (alias, option) = match link.destination() {
            rd_ast::RdLinkDestination::DisplayText { nodes } => {
                (text_contents(nodes), package.to_owned())
            }
            rd_ast::RdLinkDestination::Explicit { topic } => {
                (topic.to_string(), format!("{package}:{topic}"))
            }
            rd_ast::RdLinkDestination::Package { .. } => return None,
            _ => return None,
        };
        return matches!(
            provider.topic_exists(package, &alias),
            TopicExistence::Known(true)
        )
        .then_some((RdTag::Link, option));
    }

    let s4 = node.s4_class_link(&path)?;
    let alias = format!("{}-class", s4.class_text()?);
    matches!(
        provider.topic_exists(package, &alias),
        TopicExistence::Known(true)
    )
    .then(|| (RdTag::Link, format!("{package}:{alias}")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use rd_ast::{RdDocument, RdNode, RdTag};

    use super::*;

    fn tagged(tag: RdTag, children: Vec<RdNode>) -> RdNode {
        RdNode::tagged(tag, None, children)
    }

    fn group(text: &str) -> RdNode {
        RdNode::group(vec![RdNode::Text(text.to_owned())])
    }

    fn link(option: Option<&str>, label: &str) -> RdNode {
        RdNode::tagged(
            RdTag::Link,
            option.map(|value| vec![RdNode::Text(value.to_owned())]),
            vec![RdNode::Text(label.to_owned())],
        )
    }

    struct AliasProvider {
        aliases: BTreeSet<String>,
        available: bool,
    }

    impl AliasProvider {
        fn known(aliases: &[&str]) -> Self {
            Self {
                aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                available: true,
            }
        }

        fn unavailable() -> Self {
            Self {
                aliases: BTreeSet::new(),
                available: false,
            }
        }
    }

    impl DocumentationProvider for AliasProvider {
        fn get_topic(
            &self,
            _request: &super::super::TopicRequest,
        ) -> Result<Option<InheritableTopic>, super::super::DocumentationError> {
            unreachable!("projection tests do not fetch topics")
        }

        fn topic_exists(&self, package: &str, alias: &str) -> TopicExistence {
            assert_eq!(package, "donor");
            if self.available {
                TopicExistence::Known(self.aliases.contains(alias))
            } else {
                TopicExistence::Unavailable
            }
        }
    }

    #[test]
    fn absolutizes_supported_donor_relative_links_only() {
        let provider = AliasProvider::known(&["x", "Class-class"]);

        assert_eq!(
            absolutize_external_link(link(None, "x"), "donor", &provider),
            link(Some("donor"), "x")
        );
        assert_eq!(
            absolutize_external_link(link(Some("=x"), "label"), "donor", &provider),
            link(Some("donor:x"), "label")
        );
        assert_eq!(
            absolutize_external_link(
                tagged(RdTag::LinkS4Class, vec![RdNode::Text("Class".into())]),
                "donor",
                &provider,
            ),
            link(Some("donor:Class-class"), "Class")
        );

        for qualified in [link(Some("pkg:x"), "label"), link(Some("pkg"), "x")] {
            let original = qualified.clone();
            assert_eq!(
                absolutize_external_link(qualified, "donor", &provider),
                original
            );
        }
    }

    #[test]
    fn s4_class_link_retargets_to_donor_ignoring_an_existing_package_option() {
        // Pinned to match roxygen2's tweak_links(), not a preference of this
        // project: its \linkS4class branch rewrites the link whenever the
        // donor has a matching alias, without ever consulting an existing
        // package option on the node (unlike its \link branch, which checks
        // the option first). So \linkS4class[other]{Class} is retargeted to
        // the donor even though the author wrote a different package, and
        // this project intentionally keeps that same behavior for
        // byte-for-byte compatibility with upstream output.
        let provider = AliasProvider::known(&["Class-class"]);
        let qualified = RdNode::tagged(
            RdTag::LinkS4Class,
            Some(vec![RdNode::Text("other".into())]),
            vec![RdNode::Text("Class".into())],
        );

        assert_eq!(
            absolutize_external_link(qualified, "donor", &provider),
            link(Some("donor:Class-class"), "Class")
        );
    }

    #[test]
    fn absent_and_unavailable_aliases_leave_relative_links_unchanged() {
        let relative = link(Some("=x"), "label");
        assert_eq!(
            absolutize_external_link(relative.clone(), "donor", &AliasProvider::known(&[]),),
            relative
        );

        let relative = link(None, "x");
        assert_eq!(
            absolutize_external_link(relative.clone(), "donor", &AliasProvider::unavailable(),),
            relative
        );
    }

    #[test]
    fn projection_absolutizes_nested_links_in_params_details_and_named_sections() {
        let document = RdDocument::new(vec![
            tagged(
                RdTag::Arguments,
                vec![tagged(
                    RdTag::Item,
                    vec![
                        group("arg"),
                        RdNode::group(vec![tagged(
                            RdTag::Code,
                            vec![link(Some("=param"), "parameter")],
                        )]),
                    ],
                )],
            ),
            tagged(
                RdTag::Details,
                vec![tagged(RdTag::Emph, vec![link(None, "details")])],
            ),
            tagged(
                RdTag::Section,
                vec![
                    group("Extra"),
                    RdNode::group(vec![tagged(
                        RdTag::Code,
                        vec![link(Some("=section"), "section label")],
                    )]),
                ],
            ),
        ]);
        let provider = AliasProvider::known(&["param", "details", "section"]);

        let topic = project_external_topic("donor", "canonical", &document, &provider);
        assert_eq!(
            topic.params[0].description.value,
            InheritableContent::Rd(vec![tagged(
                RdTag::Code,
                vec![link(Some("donor:param"), "parameter")],
            )])
        );
        assert_eq!(
            topic.fields.details.unwrap().value,
            InheritableContent::Rd(vec![tagged(
                RdTag::Emph,
                vec![link(Some("donor"), "details")],
            )])
        );
        assert_eq!(
            topic.sections[0].body.value,
            InheritableContent::Rd(vec![tagged(
                RdTag::Code,
                vec![link(Some("donor:section"), "section label")],
            )])
        );
    }

    #[test]
    fn parameter_names_split_only_direct_text_and_keep_the_full_rd_label() {
        let name = vec![
            tagged(RdTag::Code, vec![RdNode::Text("x,y".into())]),
            RdNode::Text(", ".into()),
            link(Some("=z"), "z"),
            RdNode::Text(", ".into()),
            tagged(RdTag::Dots, Vec::new()),
        ];
        let document = RdDocument::new(vec![tagged(
            RdTag::Arguments,
            vec![tagged(
                RdTag::Item,
                vec![RdNode::group(name.clone()), group("description")],
            )],
        )]);

        let topic =
            project_external_topic("donor", "topic", &document, &AliasProvider::known(&["z"]));
        assert_eq!(
            topic.params[0].names,
            [
                ParamName("x,y".into()),
                ParamName("z".into()),
                ParamName("...".into())
            ]
        );
        assert_eq!(
            topic.params[0].label,
            InheritableParamLabel::Rd(vec![
                tagged(RdTag::Code, vec![RdNode::Text("x,y".into())]),
                RdNode::Text(", ".into()),
                link(Some("donor:z"), "z"),
                RdNode::Text(", ".into()),
                tagged(RdTag::Dots, Vec::new()),
            ])
        );

        let split_name = vec![
            RdNode::Text("x".into()),
            RdNode::Text(",".into()),
            RdNode::Text("y".into()),
        ];
        assert_eq!(
            parameter_names(&split_name),
            [ParamName("x".into()), ParamName("y".into())]
        );

        let protected_commas = vec![
            tagged(RdTag::Code, vec![RdNode::RCode("x,y".into())]),
            RdNode::Text(", ".into()),
            link(Some("=z,w"), "z,w"),
            RdNode::Text(", ".into()),
            RdNode::Text("before ".into()),
            RdNode::Comment("% comma, inside comment".into()),
            RdNode::Text("\nafter".into()),
            RdNode::Text(", ".into()),
            RdNode::Verb("v,w".into()),
        ];
        assert_eq!(
            parameter_names(&protected_commas),
            [
                ParamName("x,y".into()),
                ParamName("z,w".into()),
                ParamName("before \nafter".into()),
                ParamName("v,w".into()),
            ]
        );
    }

    #[test]
    fn malformed_parameter_separators_fall_back_to_generated_labels() {
        let cases = [
            (vec![RdNode::Text(",x".into())], vec![ParamName("x".into())]),
            (vec![RdNode::Text("x,".into())], vec![ParamName("x".into())]),
            (
                vec![RdNode::Text("x,,y".into())],
                vec![ParamName("x".into()), ParamName("y".into())],
            ),
            (
                vec![
                    RdNode::Comment("% comment-only, fragment".into()),
                    RdNode::Text("\n".into()),
                ],
                Vec::new(),
            ),
        ];
        for (name, expected_names) in cases {
            let document = RdDocument::new(vec![tagged(
                RdTag::Arguments,
                vec![tagged(
                    RdTag::Item,
                    vec![RdNode::group(name), group("description")],
                )],
            )]);
            let topic =
                project_external_topic("donor", "topic", &document, &AliasProvider::known(&[]));
            assert_eq!(topic.params[0].names, expected_names);
            assert_eq!(topic.params[0].label, InheritableParamLabel::Generated);
        }
    }

    #[test]
    fn raw_nodes_recurse_without_changing_payload_or_attributes() {
        let attribute = rd_ast::producer::raw_attribute(
            "custom".into(),
            rd_ast::producer::raw_object(rd_ast::RawRdValue::Symbol("value".into()), Vec::new()),
        );
        let raw = RdNode::Raw(rd_ast::producer::raw_node(
            Some("opaque".into()),
            Some(vec![link(Some("=option"), "option label")]),
            vec![link(None, "child")],
            Some(rd_ast::RawRdValue::Symbol("payload".into())),
            vec![attribute.clone()],
        ));
        let expected = RdNode::Raw(rd_ast::producer::raw_node(
            Some("opaque".into()),
            Some(vec![link(Some("donor:option"), "option label")]),
            vec![link(Some("donor"), "child")],
            Some(rd_ast::RawRdValue::Symbol("payload".into())),
            vec![attribute],
        ));

        assert_eq!(
            absolutize_external_link(raw, "donor", &AliasProvider::known(&["option", "child"]),),
            expected
        );
    }

    #[test]
    fn projects_fields_params_sections_and_canonical_provenance() {
        let raw = RdNode::Raw(rd_ast::producer::raw_node(
            Some("opaque".to_owned()),
            None,
            Vec::new(),
            None,
            Vec::new(),
        ));
        let document = RdDocument::new(vec![
            tagged(RdTag::Title, vec![RdNode::Text("Title".into())]),
            tagged(RdTag::Description, vec![raw.clone()]),
            tagged(RdTag::Details, vec![RdNode::Text("Details".into())]),
            tagged(RdTag::Value, vec![RdNode::Text("Return".into())]),
            tagged(RdTag::SeeAlso, vec![RdNode::Text("See also".into())]),
            tagged(RdTag::References, vec![RdNode::Text("References".into())]),
            tagged(RdTag::Examples, vec![RdNode::RCode("example()".into())]),
            tagged(RdTag::Author, vec![RdNode::Text("Author".into())]),
            tagged(RdTag::Source, vec![RdNode::Text("Source".into())]),
            tagged(RdTag::Note, vec![RdNode::Text("Note".into())]),
            tagged(RdTag::Format, vec![RdNode::Text("Format".into())]),
            tagged(
                RdTag::Arguments,
                vec![
                    tagged(
                        RdTag::Item,
                        vec![group("x, y"), RdNode::group(vec![raw.clone()])],
                    ),
                    tagged(
                        RdTag::Item,
                        vec![
                            RdNode::group(vec![tagged(RdTag::Dots, Vec::new())]),
                            group("dots description"),
                        ],
                    ),
                ],
            ),
            tagged(RdTag::Section, vec![group("Extra"), group("Body")]),
        ]);

        let topic =
            project_external_topic("pkg", "canonical", &document, &AliasProvider::unavailable());
        assert_eq!(
            topic.identity,
            DocumentationIdentity::External {
                package: "pkg".into(),
                topic: "canonical".into(),
            }
        );

        fn assert_content(content: &ResolvedContent, component: InheritField, expected: &[RdNode]) {
            assert_eq!(content.value, InheritableContent::Rd(expected.to_vec()));
            assert_eq!(
                content.provenance.source,
                DocumentationOrigin::External {
                    package: "pkg".into(),
                    topic: "canonical".into(),
                    component,
                }
            );
            assert!(content.provenance.requests.is_empty());
        }

        macro_rules! assert_field {
            ($field:ident, $nodes:expr, $component:expr) => {
                assert_content(
                    topic.fields.$field.as_ref().expect("projected field"),
                    $component,
                    $nodes,
                );
            };
        }
        assert_field!(title, &[RdNode::Text("Title".into())], InheritField::Title);
        assert_field!(
            description,
            std::slice::from_ref(&raw),
            InheritField::Description
        );
        assert_field!(
            details,
            &[RdNode::Text("Details".into())],
            InheritField::Details
        );
        assert_field!(
            return_value,
            &[RdNode::Text("Return".into())],
            InheritField::Return
        );
        assert_field!(
            see_also,
            &[RdNode::Text("See also".into())],
            InheritField::SeeAlso
        );
        assert_field!(
            references,
            &[RdNode::Text("References".into())],
            InheritField::References
        );
        assert_content(
            topic
                .fields
                .examples
                .as_ref()
                .expect("projected examples field"),
            InheritField::Examples,
            &[RdNode::RCode("example()".into())],
        );
        assert_field!(
            author,
            &[RdNode::Text("Author".into())],
            InheritField::Author
        );
        assert_field!(
            source,
            &[RdNode::Text("Source".into())],
            InheritField::Source
        );
        assert_field!(note, &[RdNode::Text("Note".into())], InheritField::Note);
        assert_field!(
            format,
            &[RdNode::Text("Format".into())],
            InheritField::Format
        );

        assert_eq!(
            topic.params[0].names,
            [ParamName("x".into()), ParamName("y".into())]
        );
        assert_eq!(topic.params[1].names, [ParamName("...".into())]);
        assert!(
            topic
                .params
                .iter()
                .all(|group| { group.names.iter().all(|name| !name.0.is_empty()) })
        );
        assert_content(
            &topic.params[0].description,
            InheritField::Params,
            std::slice::from_ref(&raw),
        );
        assert_eq!(topic.sections.len(), 1);
        assert_content(
            &topic.sections[0].title,
            InheritField::Sections,
            &[RdNode::Text("Extra".into())],
        );
        assert_content(
            &topic.sections[0].body,
            InheritField::Sections,
            &[RdNode::Text("Body".into())],
        );
    }
}
