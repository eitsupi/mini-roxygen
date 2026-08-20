use std::collections::BTreeMap;

use crate::arity_adapter::{PersonSection, parse_authors};
use crate::diagnostic::Diagnostics;
use crate::package::{PackageDocumentationMetadata, PackageMetadata};
use crate::r_parse::BlockTarget;
use crate::source::{SourceMap, Span};
use crate::tags::{FieldValue, ParsedTag, TagOrigin};

use super::super::{
    Alias, DocumentedBlock, PackageAuthor, PackageComment, PackageIdentity, PackageLink,
    PackageModel, PackagePerson, PackageSeeAlso, RdTopic, RdTopicKind, TopicKey, first_name,
    first_rdname, origin_span, suppresses_default_aliases,
};
use super::bindings::S7BindingResolution;
use super::block::{add_alias, merge_block, register_topic_kind};

#[derive(Debug, Default)]
pub(super) struct PackageFallbackState {
    pub(super) package_anchor: Option<Span>,
    pub(super) default_alias_anchor: Option<Span>,
    pub(super) default_aliases_enabled: bool,
    pub(super) title_suppressed: bool,
    pub(super) description_suppressed: bool,
    pub(super) see_also_suppressed: bool,
    pub(super) author_suppressed: bool,
}

pub(super) type PackageFallbackStates = BTreeMap<TopicKey, PackageFallbackState>;

#[allow(clippy::too_many_arguments)]
pub(super) fn merge_package_block(
    sources: &SourceMap,
    block: &DocumentedBlock,
    metadata: &PackageMetadata,
    package: &mut PackageModel,
    alias_owners: &mut BTreeMap<String, (TopicKey, Span)>,
    method_claims: &mut BTreeMap<(TopicKey, String, String), TagOrigin>,
    diagnostics: &mut Diagnostics,
    s7_bindings: &BTreeMap<Span, Option<S7BindingResolution>>,
    package_fallback_states: &mut PackageFallbackStates,
) {
    let default_key = format!("{}-package", metadata.package());
    let key = first_rdname(&block.tags)
        .map(|value| TopicKey(value.value.as_str().to_owned()))
        .unwrap_or_else(|| TopicKey(default_key));
    let sentinel = match block.target {
        BlockTarget::PackageDocumentation(value) => value,
        _ => unreachable!(),
    };
    record_package_fallback_suppression(block, &key, package_fallback_states);
    let state = package_fallback_states
        .get_mut(&key)
        .expect("suppression state was just recorded");
    state.package_anchor.get_or_insert(sentinel.span);
    if !suppresses_default_aliases(&block.tags) {
        state.default_alias_anchor.get_or_insert(sentinel.span);
        state.default_aliases_enabled = true;
    }
    let topic = package
        .topics
        .entry(key.clone())
        .or_insert_with(|| RdTopic::new(crate::tags::DocName(key.0.clone())));
    register_topic_kind(
        &key,
        topic,
        RdTopicKind::Package,
        sentinel.span,
        diagnostics,
    );
    if let Some(name) = first_name(&block.tags) {
        add_alias(
            &key,
            topic,
            Alias {
                name: crate::tags::DocName(name.value.as_str().to_owned()),
                span: origin_span(&name.origin),
            },
            alias_owners,
            diagnostics,
        );
    }
    merge_block(
        sources,
        block,
        &key,
        topic,
        alias_owners,
        method_claims,
        diagnostics,
        s7_bindings,
        &BTreeMap::new(),
        &BTreeMap::new(),
        false,
        &[],
    );
}

/// Adds package aliases after all ordinary topic aliases have been collected.
///
/// A package name that is also the implicit name of an ordinary topic belongs
/// to that ordinary topic. The package default is therefore omitted without a
/// diagnostic. Collisions between package topics, or with explicit package
/// aliases, continue through `add_alias` and retain the normal diagnostic.
pub(super) fn finalize_package_aliases(
    metadata: &PackageMetadata,
    package: &mut PackageModel,
    alias_owners: &mut BTreeMap<String, (TopicKey, Span)>,
    diagnostics: &mut Diagnostics,
    package_fallback_states: &PackageFallbackStates,
) {
    let package_keys = package
        .topics
        .keys()
        .filter(|key| {
            package_fallback_states
                .get(*key)
                .is_some_and(|state| state.default_aliases_enabled)
        })
        .cloned()
        .collect::<Vec<_>>();

    for key in package_keys {
        let Some(state) = package_fallback_states.get(&key) else {
            continue;
        };
        let Some(span) = state.default_alias_anchor else {
            continue;
        };
        for alias in [metadata.package().to_owned(), key.0.clone()] {
            let ordinary_owner = alias_owners
                .get(&alias)
                .filter(|(owner_key, _)| owner_key != &key)
                .and_then(|(owner_key, _)| package.topics.get(owner_key))
                .is_some_and(|topic| topic.kind != super::super::RdTopicKind::Package);
            if ordinary_owner {
                continue;
            }
            let topic = package
                .topics
                .get_mut(&key)
                .expect("package alias state must have a topic");
            add_alias(
                &key,
                topic,
                Alias {
                    name: crate::tags::DocName(alias),
                    span,
                },
                alias_owners,
                diagnostics,
            );
        }
    }
}

pub(super) fn record_package_fallback_suppression(
    block: &DocumentedBlock,
    key: &TopicKey,
    states: &mut PackageFallbackStates,
) {
    let state = states.entry(key.clone()).or_default();
    for tag in &block.tags {
        match tag {
            ParsedTag::Title(value) if matches!(value.value, FieldValue::Suppress) => {
                state.title_suppressed = true;
            }
            ParsedTag::Description(value) if matches!(value.value, FieldValue::Suppress) => {
                state.description_suppressed = true;
            }
            ParsedTag::SeeAlso(value) if matches!(value.value, FieldValue::Suppress) => {
                state.see_also_suppressed = true;
            }
            ParsedTag::Author(value) if matches!(value.value, FieldValue::Suppress) => {
                state.author_suppressed = true;
            }
            _ => {}
        }
    }
}

pub(super) fn package_authors(
    field: &str,
) -> Result<Option<PackageAuthor>, crate::arity_adapter::AuthorsParseError> {
    let people = parse_authors(field)?;
    let mut result = PackageAuthor {
        maintainers: Vec::new(),
        authors: Vec::new(),
        other_contributors: Vec::new(),
    };
    for person in &people {
        let rendered = person.render();
        let identities = person
            .comment
            .iter()
            .filter_map(|comment| {
                let label = comment.name.as_deref()?;
                let prefix = match label {
                    "ORCID" => "https://orcid.org/",
                    "ROR" => "https://ror.org/",
                    _ => return None,
                };
                Some(PackageIdentity {
                    label: label.to_owned(),
                    href: if comment.value.starts_with("http://")
                        || comment.value.starts_with("https://")
                    {
                        comment.value.clone()
                    } else {
                        format!("{prefix}{}", comment.value)
                    },
                })
            })
            .collect::<Vec<_>>();
        let comments = person
            .comment
            .iter()
            .filter(|comment| !matches!(comment.name.as_deref(), Some("ORCID" | "ROR")))
            .map(|comment| PackageComment {
                label: comment.name.clone(),
                value: comment.value.clone(),
            })
            .collect::<Vec<_>>();
        let roles = person
            .role
            .iter()
            .filter(|role| role.as_str() != "aut" && role.as_str() != "cre")
            .filter_map(|role| crate::marc_roles::role_term(role).map(str::to_owned))
            .collect();
        let value = PackagePerson {
            name: [person.given.as_deref(), person.family.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" "),
            email: person.email.clone(),
            identities,
            comments,
            roles,
        };
        match rendered.section {
            PersonSection::Maintainer => result.maintainers.push(value.clone()),
            PersonSection::Author => result.authors.push(value.clone()),
            PersonSection::OtherContributor => result.other_contributors.push(value.clone()),
        }
        if person.role.iter().any(|role| role == "cre")
            && person.role.iter().any(|role| role == "aut")
        {
            result.authors.insert(0, value);
        }
    }
    Ok((!result.maintainers.is_empty()
        || !result.authors.is_empty()
        || !result.other_contributors.is_empty())
    .then_some(result))
}

pub(super) fn package_seealso(metadata: &PackageDocumentationMetadata) -> Option<PackageSeeAlso> {
    let mut links = Vec::new();
    if let Some(urls) = metadata.url.as_deref() {
        for url in urls.split(',') {
            let url = url.trim();
            if !url.is_empty() {
                links.push(PackageLink {
                    target: url
                        .strip_prefix("https://doi.org/")
                        .unwrap_or(url)
                        .to_owned(),
                    doi: url.starts_with("https://doi.org/"),
                });
            }
        }
    }
    (!links.is_empty() || metadata.bug_reports.is_some()).then(|| PackageSeeAlso {
        urls: links,
        bug_reports: metadata.bug_reports.clone(),
    })
}
