use std::collections::{BTreeMap, BTreeSet};

use crate::arity_adapter::{BlockId, S7ClassRefusal, S7ClassRefusalReason};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::r_parse::BlockTarget;
use crate::s3_register::S3RegistrationFact;
use crate::source::{FileId, SourceMap, Span};
use crate::tags::{FieldValue, ParsedTag, TagOrigin};
use crate::usage::UsageError;

use super::super::{
    Alias, DocumentedBlock, FormalNames, InheritanceRequest, MethodDeclaration, ParamDescription,
    RdTopic, RdTopicKind, ResolvedUsage, TopicKey, TopicKindOrigin, UsageContribution,
    data_object_span, first_order, implicit_object_name, origin_span, resolve_explicit_usage,
    resolve_formal_names, resolve_usage, set_field, set_tag,
};
use super::bindings::S7BindingResolution;

/// Returns whether a block should produce an Rd topic, mirroring roxygen2's
/// `needs_doc`: the Rd-producing tag set is `description`, `param`, `return`,
/// `title`, `example`, `examples`, `name`, `rdname`, `details`, and `inherit`.
/// `describeIn` has no `ParsedTag` variant in this implementation. `@export`
/// alone is namespace-only, while `@export` together with a title produces
/// both an Rd topic and a namespace request.
pub(super) fn needs_doc(tags: &[ParsedTag]) -> bool {
    if tags.iter().any(|tag| matches!(tag, ParsedTag::NoRd(_))) {
        return false;
    }

    tags.iter().any(|tag| {
        matches!(
            tag,
            ParsedTag::Description(_)
                | ParsedTag::Param { .. }
                | ParsedTag::Return(_)
                | ParsedTag::Title(_)
                | ParsedTag::Examples(_)
                | ParsedTag::Name(_)
                | ParsedTag::RdName(_)
                | ParsedTag::Details(_)
                | ParsedTag::Inherit { .. }
                | ParsedTag::InheritSection { .. }
        )
    })
}

pub(super) fn block_sort_key(block: &DocumentedBlock) -> (bool, i64, FileId, u32, BlockId) {
    let order = first_order(&block.tags);
    (
        order.is_none(),
        order.unwrap_or(0),
        block.block.file,
        block.block_span.range.start(),
        block.block.block,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn merge_block(
    sources: &SourceMap,
    block: &DocumentedBlock,
    topic_key: &TopicKey,
    topic: &mut RdTopic,
    alias_owners: &mut BTreeMap<String, (TopicKey, Span)>,
    method_claims: &mut BTreeMap<(TopicKey, String, String), TagOrigin>,
    diagnostics: &mut Diagnostics,
    s7_bindings: &BTreeMap<Span, Option<S7BindingResolution>>,
    alias_formals: &BTreeMap<Span, FormalNames>,
    alias_function_formals: &BTreeMap<
        Span,
        Option<Result<Vec<crate::arity_adapter::Formal>, crate::arity_adapter::FormalError>>,
    >,
    lazy_data: bool,
    registrations: &[S3RegistrationFact],
) {
    topic.blocks.push(block.block);
    let is_data = matches!(block.target, BlockTarget::DataObject(_));
    if is_data {
        let span = data_object_span(&block.target)
            .expect("data object contributions have a source name span");
        register_topic_kind(topic_key, topic, RdTopicKind::Data, span, diagnostics);
        if !block
            .tags
            .iter()
            .any(|tag| matches!(tag, ParsedTag::Format(_)))
        {
            topic.missing_data_format_span.get_or_insert(span);
        }
    }
    let s7_resolution = match &block.target {
        BlockTarget::ValueAssignment(value) => s7_bindings
            .get(&value.assignment_span)
            .and_then(Option::as_ref),
        _ => None,
    };
    let s7_class = s7_resolution.and_then(|resolution| match resolution {
        S7BindingResolution::Supported(class) => Some(class),
        S7BindingResolution::Refused(_) => None,
    });
    topic.formals.push(super::super::FormalContribution {
        block: block.block,
        names: resolve_formal_names(
            &block.target,
            s7_class,
            match &block.target {
                BlockTarget::ValueAssignment(value) => alias_formals.get(&value.assignment_span),
                _ => None,
            },
        ),
    });

    // This single-fill policy deliberately diverges from roxygen2, which
    // concatenates scalar prose and examples contributions. A second
    // contribution is surfaced as an actionable source diagnostic instead of
    // silently depending on source order.
    let mut block_slots = BTreeSet::new();
    let mut block_slot_origins = BTreeMap::new();
    let mut explicit_usage_origin = None;
    let mut method = None;
    for tag in &block.tags {
        match tag {
            ParsedTag::Name(value) => {
                if !block_slots.insert("name") {
                    super::super::emit_duplicate(
                        diagnostics,
                        "name",
                        value.origin.clone(),
                        block_slot_origins.get("name").cloned(),
                        super::super::DuplicateSlotKind::BlockLocal,
                    );
                } else {
                    block_slot_origins.insert("name", value.origin.clone());
                }
            }
            ParsedTag::RdName(value) => {
                if !block_slots.insert("rdname") {
                    super::super::emit_duplicate(
                        diagnostics,
                        "rdname",
                        value.origin.clone(),
                        block_slot_origins.get("rdname").cloned(),
                        super::super::DuplicateSlotKind::BlockLocal,
                    );
                } else {
                    block_slot_origins.insert("rdname", value.origin.clone());
                }
            }
            ParsedTag::Title(value) => set_field(
                "title",
                &mut topic.title,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Description(value) => set_field(
                "description",
                &mut topic.description,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            // Intro trailing paragraphs and explicit @details are composed by
            // tags::intro before this layer sees the block. The resulting
            // single Details tag deliberately does not violate the per-topic
            // single-slot rule: neither part was silently discarded.
            ParsedTag::Details(value) => set_field(
                "details",
                &mut topic.details,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Return(value) => set_field(
                "return",
                &mut topic.return_value,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Examples(value) => set_tag(
                "examples",
                &mut topic.examples,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Usage(value) => {
                if let Some(previous) = explicit_usage_origin.clone() {
                    super::super::emit_duplicate(
                        diagnostics,
                        "usage",
                        value.origin.clone(),
                        Some(previous),
                        super::super::DuplicateSlotKind::BlockLocal,
                    );
                } else {
                    explicit_usage_origin = Some(value.origin.clone());
                }
            }
            ParsedTag::Order { origin, .. } => {
                if !block_slots.insert("order") {
                    super::super::emit_duplicate(
                        diagnostics,
                        "order",
                        origin.clone(),
                        block_slot_origins.get("order").cloned(),
                        super::super::DuplicateSlotKind::BlockLocal,
                    );
                } else {
                    block_slot_origins.insert("order", origin.clone());
                }
            }
            ParsedTag::Method {
                generic,
                class,
                origin,
            } => {
                if !block_slots.insert("method") {
                    super::super::emit_duplicate(
                        diagnostics,
                        "method",
                        origin.clone(),
                        block_slot_origins.get("method").cloned(),
                        super::super::DuplicateSlotKind::BlockLocal,
                    );
                } else {
                    block_slot_origins.insert("method", origin.clone());
                    let declaration = MethodDeclaration {
                        generic: generic.clone(),
                        class: class.clone(),
                        origin: origin.clone(),
                    };
                    let method_key = (
                        topic_key.clone(),
                        generic.value.clone(),
                        class.value.clone(),
                    );
                    if let Some(previous) = method_claims.get(&method_key) {
                        super::super::emit_duplicate_method(
                            diagnostics,
                            generic,
                            class,
                            origin.clone(),
                            previous.clone(),
                        );
                    } else {
                        method_claims.insert(method_key, origin.clone());
                    }
                    method = Some(declaration);
                }
            }
            ParsedTag::Param {
                names,
                description,
                origin,
            } => {
                for name in names {
                    if let Some(previous) = topic
                        .params
                        .iter()
                        .find(|previous| previous.name.0 == name.value.0)
                    {
                        diagnostics.push(
                            Diagnostic::new(
                                Severity::Error,
                                DiagnosticCode::ConflictingParamDescription,
                                format!(
                                    "parameter `{}` has more than one description",
                                    name.value.0
                                ),
                                Label::new(name.span, "conflicting parameter description"),
                            )
                            .with_secondary(Label::new(
                                origin_span(&previous.origin),
                                "first parameter description",
                            )),
                        );
                    } else {
                        topic.params.push(ParamDescription {
                            name: name.value.clone(),
                            description: description.clone(),
                            origin: origin.clone(),
                        });
                    }
                }
            }
            ParsedTag::Section {
                title,
                body,
                origin,
            } => {
                // Keep all local sections until the shared inheritance/lowering
                // boundary. Their semantic identity depends on the same
                // Markdown link and inline-R context used by final Rd output,
                // which is not available while blocks are merged.
                topic.sections.push(super::super::NamedSection {
                    title: title.clone(),
                    body: body.clone(),
                    origin: origin.clone(),
                });
            }
            ParsedTag::Aliases(directive) => {
                for value in &directive.value.explicit {
                    add_alias(
                        topic_key,
                        topic,
                        Alias {
                            name: value.value.clone(),
                            span: value.span,
                        },
                        alias_owners,
                        diagnostics,
                    );
                }
            }
            ParsedTag::Keywords(directive) => {
                let FieldValue::Emit(values) = &directive.value else {
                    continue;
                };
                for value in values {
                    if !topic.keywords.iter().any(|seen| seen == &value.value) {
                        topic.keywords.push(value.value.clone());
                    }
                }
            }
            ParsedTag::Inherit {
                target,
                fields,
                origin,
            } => topic.inheritance.push(InheritanceRequest::Inherit {
                target: target.clone(),
                fields: fields.clone(),
                origin: origin.clone(),
            }),
            ParsedTag::InheritParams {
                target,
                selection,
                origin,
            } => topic.inheritance.push(InheritanceRequest::InheritParams {
                target: target.clone(),
                selection: selection.clone(),
                origin: origin.clone(),
            }),
            ParsedTag::InheritSection {
                target,
                title,
                origin,
            } => topic.inheritance.push(InheritanceRequest::InheritSection {
                target: target.clone(),
                title: title.clone(),
                origin: origin.clone(),
            }),
            ParsedTag::Namespace(_) | ParsedTag::NoRd(_) => {}
            ParsedTag::SeeAlso(value) => set_field(
                "seealso",
                &mut topic.see_also,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::References(value) => set_field(
                "references",
                &mut topic.references,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Note(value) => set_field(
                "note",
                &mut topic.note,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Format(value) => set_field(
                "format",
                &mut topic.format,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Source(value) => set_field(
                "source",
                &mut topic.source,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Author(value) => set_field(
                "author",
                &mut topic.author,
                value.clone(),
                &mut block_slots,
                &mut block_slot_origins,
                diagnostics,
            ),
            ParsedTag::Unsupported(_) | ParsedTag::Unknown(_) => {}
        }
    }
    if method.is_none()
        && let Some(name) = implicit_object_name(&block.target)
        && let Some(registration) =
            super::bindings::registration_for_target(registrations, name.as_str())
    {
        method = Some(super::bindings::registration_method(registration));
    }

    if is_data
        && !block.tags.iter().any(|tag| {
            matches!(
                tag,
                ParsedTag::Keywords(value) if matches!(value.value, FieldValue::Suppress)
            )
        })
        && !topic.keywords.iter().any(|keyword| keyword.0 == "datasets")
    {
        topic.keywords.push(crate::tags::Keyword("datasets".into()));
    }

    let usage = if explicit_usage_origin.is_some() {
        block.tags.iter().find_map(|tag| match tag {
            ParsedTag::Usage(value) => Some(resolve_explicit_usage(value)),
            _ => None,
        })
    } else {
        if let Some(S7BindingResolution::Refused(refusal)) = s7_resolution {
            diagnostics.push(unsupported_s7_diagnostic(refusal, &block.target));
        }
        let alias_function_formals = match &block.target {
            BlockTarget::ValueAssignment(value) => alias_function_formals
                .get(&value.assignment_span)
                .and_then(Option::as_ref),
            _ => None,
        };
        match resolve_usage(
            &block.target,
            sources,
            s7_class,
            alias_function_formals,
            lazy_data,
        ) {
            Ok(Some(usage)) => Some(ResolvedUsage::Generated(usage)),
            Ok(None) => Some(ResolvedUsage::Absent),
            Err(error) => {
                diagnostics.push(usage_error_diagnostic(error, block.block_span));
                Some(ResolvedUsage::Absent)
            }
        }
    }
    .expect("each contributing block produces one usage contribution");
    topic.usages.push(UsageContribution {
        block: block.block,
        block_span: block.block_span,
        object: implicit_object_name(&block.target).cloned(),
        method: if is_data { None } else { method },
        usage,
    });
}

pub(super) fn is_function_target(target: &BlockTarget) -> bool {
    matches!(target, BlockTarget::FunctionAssignment(_))
}

pub(super) fn has_export_s3_method_null(tags: &[ParsedTag]) -> bool {
    tags.iter().any(|tag| {
        matches!(
            tag,
            ParsedTag::Namespace(crate::tags::NamespaceTag::ExportS3Method(value))
                if value.value.as_str().trim() == "NULL"
        )
    })
}

pub(super) fn export_s3_method_null_span(tags: &[ParsedTag]) -> Option<Span> {
    tags.iter().find_map(|tag| match tag {
        ParsedTag::Namespace(crate::tags::NamespaceTag::ExportS3Method(value))
            if value.value.as_str().trim() == "NULL" =>
        {
            Some(origin_span(&value.origin))
        }
        _ => None,
    })
}

pub(super) fn has_namespace_export_or_suppression(tags: &[ParsedTag]) -> bool {
    tags.iter().any(|tag| {
        matches!(
            tag,
            ParsedTag::Namespace(
                crate::tags::NamespaceTag::Export(_) | crate::tags::NamespaceTag::ExportS3Method(_)
            )
        )
    })
}

pub(super) fn register_topic_kind(
    topic_key: &TopicKey,
    topic: &mut RdTopic,
    kind: RdTopicKind,
    span: Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(first) = topic.kind_origin else {
        topic.kind_origin = Some(TopicKindOrigin { kind, span });
        topic.kind = kind;
        return;
    };

    if first.kind == kind {
        topic.kind = if topic.kind_conflict_reported {
            RdTopicKind::Package
        } else {
            kind
        };
        return;
    }

    if !topic.kind_conflict_reported {
        diagnostics.push(
            Diagnostic::new(
                Severity::Error,
                DiagnosticCode::ConflictingTopicKind,
                format!(
                    "topic `{}` mixes package and data documentation contributions with incompatible document types",
                    topic_key.as_str()
                ),
                Label::new(span, "conflicting topic kind contribution"),
            )
            .with_secondary(Label::new(
                first.span,
                "first contribution establishing the other topic kind",
            ))
            .with_help("use distinct @rdname values or correct the contribution kind"),
        );
        topic.kind_conflict_reported = true;
    }

    topic.kind =
        if matches!(first.kind, RdTopicKind::Package) || matches!(kind, RdTopicKind::Package) {
            RdTopicKind::Package
        } else {
            RdTopicKind::Data
        };
}

pub(in crate::model) fn add_alias(
    topic_key: &TopicKey,
    topic: &mut RdTopic,
    alias: Alias,
    alias_owners: &mut BTreeMap<String, (TopicKey, Span)>,
    diagnostics: &mut Diagnostics,
) {
    if topic.aliases.iter().any(|seen| seen.name == alias.name) {
        return;
    }

    if let Some((owner_key, owner_span)) = alias_owners.get(alias.name.0.as_str())
        && owner_key != topic_key
    {
        diagnostics.push(
            Diagnostic::new(
                Severity::Error,
                DiagnosticCode::ConflictingAlias,
                format!("alias `{}` is claimed by more than one topic", alias.name.0),
                Label::new(alias.span, "alias claimed by this topic"),
            )
            .with_secondary(Label::new(*owner_span, "alias claimed by another topic")),
        );
    }

    alias_owners
        .entry(alias.name.0.clone())
        .or_insert_with(|| (topic_key.clone(), alias.span));
    topic.aliases.push(alias);
}

pub(super) fn usage_error_diagnostic(error: UsageError, block_span: Span) -> Diagnostic {
    let span = match error {
        UsageError::InvalidFormals(_) => block_span,
        UsageError::InvalidReplacementSignature { name_span: span }
        | UsageError::UndecodableFormalName { span, .. }
        | UsageError::UnresolvableSourceSpan { span }
        | UsageError::UnsafeDefaultComment { span } => span,
    };
    Diagnostic::new(
        Severity::Error,
        DiagnosticCode::UsageGenerationFailed,
        format!("could not generate function usage: {error:?}"),
        Label::new(span, "usage generation failed"),
    )
}

fn unsupported_s7_diagnostic(refusal: &S7ClassRefusal, target: &BlockTarget) -> Diagnostic {
    let primary = match target {
        BlockTarget::ValueAssignment(value) => value.value_span,
        _ => refusal.span,
    };
    let message = match refusal.reason {
        S7ClassRefusalReason::ComputedClassName => {
            "S7 new_class requires a literal first class-name argument"
        }
        S7ClassRefusalReason::MissingConstructor => {
            "S7 new_class requires a direct constructor = function(...) argument"
        }
        S7ClassRefusalReason::ComputedConstructor => {
            "S7 new_class constructor must be a direct function(...) argument"
        }
    };
    let diagnostic = Diagnostic::new(
        Severity::Error,
        DiagnosticCode::UnsupportedS7Constructor,
        message,
        Label::new(primary, "unsupported S7 constructor metadata"),
    );
    if primary == refusal.span {
        diagnostic
    } else {
        diagnostic.with_secondary(Label::new(refusal.span, "originating S7 class refusal"))
    }
}
