//! Merges documented blocks into ordered package topics and namespace requests.
//!
//! The facade owns traversal order and delegates cohesive binding, package,
//! and block responsibilities to the sibling modules in this directory.

use std::collections::BTreeMap;

use crate::arity_adapter::RName;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label};
use crate::package::PackageMetadata;
use crate::r_parse::{AssociationRefusal, BindingFact, BlockTarget};
use crate::s3_register::S3RegistrationFact;
use crate::source::{SourceMap, Span};
use crate::tags::{ParsedTag, TagOrigin};

use super::{
    Alias, DocumentedBlock, MethodDeclaration, ModelOutput, NamespaceRequest,
    PackageMetadataDiagnosticState, PackageModel, RdTopic, RdTopicKind, TopicKey, data_object_name,
    emit_data_name_diagnostic, emit_missing_identity, emit_package_documentation_diagnostic,
    first_name, first_rdname, has_no_rd, implicit_object_name, implicit_object_span,
    is_refused_or_null, origin_span, suppresses_default_aliases,
};

mod bindings;
mod block;
mod package;

#[cfg(test)]
mod tests;

/// Builds package topics from already parsed documentation blocks.
#[must_use]
#[cfg(test)]
pub fn build_package_model(sources: &SourceMap, mut blocks: Vec<DocumentedBlock>) -> ModelOutput {
    blocks.sort_by_key(block::block_sort_key);
    build_package_model_inner(sources, &mut blocks, None, Vec::new(), Vec::new())
}

/// Builds package topics while retaining package-wide top-level binding facts.
#[must_use]
#[cfg(test)]
pub fn build_package_model_with_bindings(
    sources: &SourceMap,
    mut blocks: Vec<DocumentedBlock>,
    bindings: Vec<BindingFact>,
) -> ModelOutput {
    blocks.sort_by_key(block::block_sort_key);
    build_package_model_inner(sources, &mut blocks, None, bindings, Vec::new())
}

/// Builds package topics while applying DESCRIPTION defaults to a package topic.
#[must_use]
#[cfg(test)]
pub fn build_package_model_with_metadata(
    sources: &SourceMap,
    mut blocks: Vec<DocumentedBlock>,
    metadata: &PackageMetadata,
) -> ModelOutput {
    blocks.sort_by_key(block::block_sort_key);
    build_package_model_inner(sources, &mut blocks, Some(metadata), Vec::new(), Vec::new())
}

/// Builds package topics with metadata and package-wide top-level bindings.
#[must_use]
#[cfg(test)]
pub fn build_package_model_with_metadata_and_bindings(
    sources: &SourceMap,
    mut blocks: Vec<DocumentedBlock>,
    metadata: &PackageMetadata,
    bindings: Vec<BindingFact>,
) -> ModelOutput {
    blocks.sort_by_key(block::block_sort_key);
    build_package_model_inner(sources, &mut blocks, Some(metadata), bindings, Vec::new())
}

/// Builds package topics with metadata, bindings, and static S3 registrar facts.
#[must_use]
pub fn build_package_model_with_metadata_bindings_and_registrations(
    sources: &SourceMap,
    mut blocks: Vec<DocumentedBlock>,
    metadata: &PackageMetadata,
    bindings: Vec<BindingFact>,
    registrations: Vec<S3RegistrationFact>,
) -> ModelOutput {
    blocks.sort_by_key(block::block_sort_key);
    build_package_model_inner(
        sources,
        &mut blocks,
        Some(metadata),
        bindings,
        registrations,
    )
}

fn build_package_model_inner(
    sources: &SourceMap,
    blocks: &mut [DocumentedBlock],
    metadata: Option<&PackageMetadata>,
    bindings: Vec<BindingFact>,
    registrations: Vec<S3RegistrationFact>,
) -> ModelOutput {
    let s7_bindings = bindings::s7_binding_results(&bindings);
    let collate = metadata.is_some_and(PackageMetadata::has_collate_directive);
    let alias_formals = bindings::alias_formal_results(&bindings, sources, collate);
    let alias_function_formals =
        bindings::alias_function_formal_results(&bindings, sources, collate);
    let mut package = PackageModel {
        bindings,
        registrations,
        collate,
        ..PackageModel::default()
    };
    let mut diagnostics = Diagnostics::new();
    bindings::validate_registration_targets(
        &package.registrations,
        &package.bindings,
        &mut diagnostics,
    );
    let mut alias_owners: BTreeMap<String, (TopicKey, Span)> = BTreeMap::new();
    let mut method_claims: BTreeMap<(TopicKey, String, String), TagOrigin> = BTreeMap::new();
    let mut package_fallback_states = package::PackageFallbackStates::default();

    for block_ref in blocks.iter() {
        let implicit_object = implicit_object_name(&block_ref.target);
        let explicit_method = block_ref.tags.iter().find_map(|tag| match tag {
            ParsedTag::Method {
                generic,
                class,
                origin,
            } => Some(MethodDeclaration {
                generic: generic.clone(),
                class: class.clone(),
                origin: origin.clone(),
            }),
            _ => None,
        });
        let registration = implicit_object.and_then(|name| {
            bindings::registration_for_target(&package.registrations, name.as_str())
        });
        let matching_registrations = implicit_object
            .map(|name| bindings::registration_matches(&package.registrations, name.as_str()))
            .unwrap_or_default();
        let method = explicit_method
            .clone()
            .or_else(|| registration.map(bindings::registration_method));
        for tag in &block_ref.tags {
            if let ParsedTag::Namespace(tag) = tag {
                package.namespace.push(NamespaceRequest {
                    block: block_ref.block,
                    tag: tag.clone(),
                    object: implicit_object.cloned(),
                    object_is_function: matches!(
                        block_ref.target,
                        BlockTarget::FunctionAssignment(_)
                    ),
                    object_spelling: implicit_object_span(&block_ref.target),
                    method: explicit_method.clone(),
                });
            }
        }

        if has_no_rd(&block_ref.tags) {
            continue;
        }
        let is_package = matches!(block_ref.target, BlockTarget::PackageDocumentation(_));
        if let BlockTarget::Refused(
            refusal @ (AssociationRefusal::UndecodableDataName { .. }
            | AssociationRefusal::EmptyDataName { .. }),
        ) = &block_ref.target
        {
            emit_data_name_diagnostic(&mut diagnostics, refusal);
            continue;
        }
        if !is_package && !block::needs_doc(&block_ref.tags) {
            continue;
        }
        if block::is_function_target(&block_ref.target)
            && block::has_export_s3_method_null(&block_ref.tags)
            && method.is_none()
            && matching_registrations.is_empty()
        {
            let tag_span =
                block::export_s3_method_null_span(&block_ref.tags).unwrap_or(block_ref.block_span);
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticCode::UnresolvedS3MethodMetadata.default_severity(),
                    DiagnosticCode::UnresolvedS3MethodMetadata,
                    "@exportS3Method NULL has no statically known generic and class",
                    Label::new(tag_span, "S3 method metadata is unresolved"),
                )
                .with_help(
                    "add a matching registrar configuration or an explicit @method <generic> <class>",
                ),
            );
        }
        if !is_package && explicit_method.is_none() && matching_registrations.len() > 1 {
            let name = implicit_object
                .map(RName::as_str)
                .unwrap_or("documented method");
            let mut diagnostic = Diagnostic::new(
                DiagnosticCode::AmbiguousS3Registration.default_severity(),
                DiagnosticCode::AmbiguousS3Registration,
                format!("documented function `{name}` maps to multiple S3 registration pairs"),
                Label::new(
                    implicit_object_span(&block_ref.target).unwrap_or(block_ref.block_span),
                    "multiple registration facts claim this target",
                ),
            )
            .with_help(
                "add an explicit @method <generic> <class> or separate the registrations across distinct targets",
            );
            for registration in &matching_registrations {
                diagnostic = diagnostic.with_secondary(Label::new(
                    registration.span,
                    format!(
                        "registration pair is {}.{}",
                        registration.generic, registration.class
                    ),
                ));
            }
            diagnostics.push(diagnostic);
        }
        if !is_package
            && !matching_registrations.is_empty()
            && (matching_registrations.len() == 1 || explicit_method.is_some())
            && !block::has_namespace_export_or_suppression(&block_ref.tags)
        {
            let name = implicit_object
                .map(RName::as_str)
                .unwrap_or("registered method");
            let registration_context = registration.map_or_else(
                || "multiple registration pairs".to_owned(),
                |registration| format!("{}.{}", registration.generic, registration.class),
            );
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticCode::UnexportedS3Method.default_severity(),
                    DiagnosticCode::UnexportedS3Method,
                    format!("documented S3 method `{name}` has no export or NULL suppression"),
                    Label::new(
                        implicit_object_span(&block_ref.target).unwrap_or(block_ref.block_span),
                        "registered S3 method is not exported",
                    ),
                )
                .with_help("add @exportS3Method NULL or an export tag")
                .with_context("registration", registration_context),
            );
        }

        if is_package {
            let Some(metadata) = metadata else {
                emit_package_documentation_diagnostic(
                    &mut diagnostics,
                    match block_ref.target {
                        BlockTarget::PackageDocumentation(sentinel) => sentinel,
                        _ => unreachable!(),
                    },
                );
                continue;
            };
            package::merge_package_block(
                sources,
                block_ref,
                metadata,
                &mut package,
                &mut alias_owners,
                &mut method_claims,
                &mut diagnostics,
                &s7_bindings,
                &mut package_fallback_states,
            );
            continue;
        }

        let explicit_name = first_name(&block_ref.tags);
        let implicit_name = implicit_object;
        if is_refused_or_null(&block_ref.target) && explicit_name.is_none() {
            emit_missing_identity(&mut diagnostics, block_ref);
            continue;
        }
        let primary = explicit_name
            .map(|value| crate::tags::DocName(value.value.as_str().to_owned()))
            .or_else(|| implicit_name.map(|name| crate::tags::DocName(name.as_str().to_owned())))
            .or_else(|| {
                data_object_name(&block_ref.target)
                    .map(|name| crate::tags::DocName(name.as_str().to_owned()))
            });
        let Some(primary) = primary else {
            emit_missing_identity(&mut diagnostics, block_ref);
            continue;
        };
        let key = first_rdname(&block_ref.tags)
            .map(|value| TopicKey(value.value.as_str().to_owned()))
            .unwrap_or_else(|| TopicKey(primary.0.clone()));
        package::record_package_fallback_suppression(block_ref, &key, &mut package_fallback_states);
        let topic = package
            .topics
            .entry(key.clone())
            .or_insert_with(|| RdTopic::new(crate::tags::DocName(key.0.clone())));
        if topic.blocks.is_empty() {
            topic.name = primary.clone();
        }
        if !suppresses_default_aliases(&block_ref.tags) {
            let primary_span = explicit_name
                .map(|value| origin_span(&value.origin))
                .or_else(|| implicit_object_span(&block_ref.target))
                .or_else(|| super::data_object_span(&block_ref.target))
                .expect("a topic identity has either an explicit or implicit source span");
            block::add_alias(
                &key,
                topic,
                Alias {
                    name: primary.clone(),
                    span: primary_span,
                },
                &mut alias_owners,
                &mut diagnostics,
            );
            let implicit_alias_name = implicit_name
                .map(RName::as_str)
                .or_else(|| data_object_name(&block_ref.target).map(|name| name.as_str()));
            if let Some(implicit_name) = implicit_alias_name
                && explicit_name.is_some_and(|name| name.value.as_str() != implicit_name)
            {
                block::add_alias(
                    &key,
                    topic,
                    Alias {
                        name: crate::tags::DocName(implicit_name.to_owned()),
                        span: implicit_object_span(&block_ref.target)
                            .or_else(|| super::data_object_span(&block_ref.target))
                            .expect("an implicit object name has a source span"),
                    },
                    &mut alias_owners,
                    &mut diagnostics,
                );
            }
        }
        block::merge_block(
            sources,
            block_ref,
            &key,
            topic,
            &mut alias_owners,
            &mut method_claims,
            &mut diagnostics,
            &s7_bindings,
            &alias_formals,
            &alias_function_formals,
            metadata.is_some_and(PackageMetadata::lazy_data),
            &package.registrations,
        );
    }

    if let Some(metadata) = metadata {
        package::finalize_package_aliases(
            metadata,
            &mut package,
            &mut alias_owners,
            &mut diagnostics,
            &package_fallback_states,
        );
    }

    if let Some(metadata) = metadata {
        for (key, topic) in &mut package.topics {
            if topic.kind != RdTopicKind::Package {
                continue;
            }
            let state = package_fallback_states
                .get(key)
                .expect("package topics must have package fallback state");
            let anchor = state
                .package_anchor
                .expect("package topics must have a package diagnostic anchor");
            topic.description_suppressed = state.description_suppressed;
            if topic.title.is_none()
                && !state.title_suppressed
                && let Some(title) = metadata.documentation().title.as_deref()
            {
                let value = format!("{}: {title}", metadata.package());
                topic.title = Some(crate::tags::TagValue {
                    value: crate::tags::MarkdownText::new(crate::tags::SourcedText::synthetic(
                        value, anchor,
                    )),
                    origin: TagOrigin::Implicit { intro_span: anchor },
                });
            }
            let missing_description = topic.description.is_none()
                && !state.description_suppressed
                && metadata.documentation().description.is_none();
            if topic.description.is_none()
                && !state.description_suppressed
                && let Some(description) = metadata.documentation().description.as_deref()
            {
                topic.description = Some(crate::tags::TagValue {
                    value: crate::tags::MarkdownText::new(crate::tags::SourcedText::synthetic(
                        description,
                        anchor,
                    )),
                    origin: TagOrigin::Implicit { intro_span: anchor },
                });
            }
            if topic.see_also.is_none() && !state.see_also_suppressed {
                topic.package_see_also = package::package_seealso(metadata.documentation());
            }
            let mut authors_parse_error = None;
            if topic.author.is_none()
                && !state.author_suppressed
                && let Some(authors) = metadata.documentation().authors_r.as_deref()
            {
                match package::package_authors(authors) {
                    Ok(rendered) => topic.package_author = rendered,
                    Err(error) => authors_parse_error = Some(error),
                }
            }
            topic.package_metadata_diagnostics = Some(PackageMetadataDiagnosticState {
                anchor,
                missing_description,
                authors_parse_error,
            });
        }
    }

    crate::namespace::classify_usage_methods(
        &mut package,
        sources,
        &crate::namespace::EmptyS3GenericProvider,
    );

    ModelOutput {
        package,
        diagnostics,
    }
}
