use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::markdown_conversion::markdown_section_key;
use crate::markdown_conversion::section_key::SectionTitleKey;
use crate::model::{InheritanceRequest, PackageModel, TopicKey};
use crate::rd::RdLinkResolver;
use crate::source::Span;
use crate::tags::{InheritField, InheritTarget, TopicRef};

use super::graph::{CycleReport, GraphAnalysis, analyze_inheritance_graph};
use super::merge::{
    emit_missing_params, local_only_topic, merge_inherited_section, merge_params, merge_sections,
    merge_single_field, origin_span, request_origin, request_selects_field, requested_fields,
    topic_as_inheritable,
};
use super::policy::InheritanceOutput;
use super::policy::{ExternalInheritancePolicy, ExternalPolicySource, InheritanceOptions};
use super::provider::{
    DocumentationError, DocumentationIdentity, DocumentationProvider, LocalLookupError,
    TopicRequest, lookup_local_topic,
};
use super::types::{InheritableTopic, ResolvedPackageModel, ResolvedRdTopic};

#[derive(Debug, Clone)]
pub(super) struct PreparedRequest {
    pub(super) request: InheritanceRequest,
    target: PreparedTarget,
    pub(super) effective_fields: Vec<InheritField>,
}

#[derive(Debug, Clone)]
enum PreparedTarget {
    Local(TopicKey),
    External {
        package: String,
        topic: String,
    },
    Invalid {
        span: Span,
        message: &'static str,
        target: String,
    },
    Suppressed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TargetIdentity {
    Local(TopicKey),
    External {
        package: String,
        topic: String,
    },
    Invalid {
        target: String,
        message: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RequestIdentity {
    Fields {
        target: TargetIdentity,
        fields: BTreeSet<InheritField>,
    },
    Params {
        target: TargetIdentity,
        selection: Option<Vec<SelectorIdentity>>,
    },
    Section {
        target: TargetIdentity,
        title: SectionTitleKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SelectorIdentity {
    excluded: bool,
    name: String,
}

fn deduplicate_prepared_requests(
    requests: Vec<PreparedRequest>,
    diagnostics: &mut Diagnostics,
    current_package: Option<&str>,
    links: &dyn RdLinkResolver,
    inline_r_session: &crate::inline_r::InlineRSession<'_>,
) -> Vec<PreparedRequest> {
    let mut seen = BTreeMap::<RequestIdentity, crate::tags::TagOrigin>::new();
    let mut retained = Vec::with_capacity(requests.len());
    for request in requests {
        let Some(identity) = request_identity(&request, current_package, links, inline_r_session)
        else {
            retained.push(request);
            continue;
        };
        let origin = request_origin(&request.request);
        if let Some(first) = seen.get(&identity) {
            diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    DiagnosticCode::DuplicateInheritanceRequest,
                    "duplicate inheritance request; only the first request is applied",
                    Label::new(origin_span(&origin), "later identical inheritance request"),
                )
                .with_secondary(Label::new(
                    origin_span(first),
                    "first identical inheritance request",
                )),
            );
        } else {
            seen.insert(identity, origin);
            retained.push(request);
        }
    }
    retained
}

fn request_identity(
    request: &PreparedRequest,
    current_package: Option<&str>,
    links: &dyn RdLinkResolver,
    inline_r_session: &crate::inline_r::InlineRSession<'_>,
) -> Option<RequestIdentity> {
    let target = match &request.target {
        PreparedTarget::Local(key) => TargetIdentity::Local(key.clone()),
        PreparedTarget::External { package, topic } => TargetIdentity::External {
            package: package.clone(),
            topic: topic.clone(),
        },
        PreparedTarget::Invalid {
            target, message, ..
        } => TargetIdentity::Invalid {
            target: target.clone(),
            message,
        },
        PreparedTarget::Suppressed => return None,
    };
    match &request.request {
        InheritanceRequest::Inherit { fields, .. } => Some(RequestIdentity::Fields {
            target,
            fields: requested_fields(fields).into_iter().collect(),
        }),
        InheritanceRequest::InheritParams { selection, .. } => Some(RequestIdentity::Params {
            target,
            selection: selection.as_ref().map(|selection| {
                selection
                    .selectors
                    .iter()
                    .map(|selector| match selector {
                        crate::tags::ArgSelector::Name(name) => SelectorIdentity {
                            excluded: false,
                            name: name.value.0.clone(),
                        },
                        crate::tags::ArgSelector::Exclude(name) => SelectorIdentity {
                            excluded: true,
                            name: name.value.0.clone(),
                        },
                    })
                    .collect()
            }),
        }),
        InheritanceRequest::InheritSection { title, .. } => Some(RequestIdentity::Section {
            target,
            title: markdown_section_key(
                &title.value,
                current_package,
                links,
                Some(inline_r_session),
            ),
        }),
    }
}

pub(super) struct Resolver<'a> {
    pub(super) package: &'a PackageModel,
    pub(super) current_package: Option<&'a str>,
    pub(super) links: &'a dyn RdLinkResolver,
    pub(super) inline_r_session: &'a crate::inline_r::InlineRSession<'a>,
    pub(super) provider: &'a dyn DocumentationProvider,
    pub(super) options: &'a InheritanceOptions,
    pub(super) normalized: BTreeMap<TopicKey, Vec<PreparedRequest>>,
    pub(super) memo: BTreeMap<TopicKey, ResolvedRdTopic>,
    pub(super) external_memo:
        BTreeMap<DocumentationIdentity, Result<Option<InheritableTopic>, DocumentationError>>,
    pub(super) diagnostics: Diagnostics,
}

/// Resolves all local inheritance and treats external inheritance as an
/// additive, policy-controlled provider lookup.
///
/// `current_package` and `links` are the Markdown rendering context. Callers
/// must pass the same pair to [`crate::rd::build_rd_with_context`] so section
/// lookup observes the labels that the final Rd conversion renders.
#[must_use]
#[cfg(test)]
pub fn resolve_inheritance(
    package: &PackageModel,
    current_package: Option<&str>,
    links: &dyn RdLinkResolver,
    provider: &dyn DocumentationProvider,
    options: &InheritanceOptions,
) -> InheritanceOutput {
    let substitutions = crate::inline_r::InlineRSubstitutions::builtins()
        .expect("built-in substitutions should be valid");
    let usage = crate::inline_r::InlineRUsage::new();
    let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
    resolve_inheritance_with_substitutions(
        package,
        current_package,
        links,
        provider,
        options,
        &session,
    )
}

/// Resolves inheritance against a session the caller also uses for the Rd
/// build.
///
/// Crate-private on purpose, for the same reason as `rd::build_rd_with_context`:
/// the session records which substitutions were matched, and that record is only
/// meaningful once every stage of one documentation run has shared it.
pub(crate) fn resolve_inheritance_with_substitutions(
    package: &PackageModel,
    current_package: Option<&str>,
    links: &dyn RdLinkResolver,
    provider: &dyn DocumentationProvider,
    options: &InheritanceOptions,
    inline_r_session: &crate::inline_r::InlineRSession<'_>,
) -> InheritanceOutput {
    let mut resolver = Resolver {
        package,
        current_package,
        links,
        inline_r_session,
        provider,
        options,
        normalized: BTreeMap::new(),
        memo: BTreeMap::new(),
        external_memo: BTreeMap::new(),
        diagnostics: Diagnostics::new(),
    };
    resolver.prepare();
    let graph = resolver.graph();
    let cyclic = graph.cyclic_nodes.clone();
    for component in &graph.components {
        if component.is_cyclic() {
            for key in &component.nodes {
                let topic = resolver
                    .package
                    .topics
                    .get(key)
                    .expect("graph nodes come from package topics")
                    .clone();
                let resolved = resolver.local_only_topic(key, &topic);
                resolver.memo.insert(key.clone(), resolved);
            }
            resolver.emit_cycles(&component.nodes, component.cycle.as_ref());
        }
    }
    for key in graph.dependency_order {
        if cyclic.contains(&key) {
            continue;
        }
        resolver.resolve_local(&key);
    }
    let topics = resolver
        .package
        .topics
        .keys()
        .filter_map(|key| resolver.memo.remove(key).map(|topic| (key.clone(), topic)))
        .collect();
    InheritanceOutput {
        package: ResolvedPackageModel {
            topics,
            namespace: package.namespace.clone(),
        },
        diagnostics: resolver.diagnostics,
    }
}

impl Resolver<'_> {
    pub(super) fn prepare(&mut self) {
        for (key, topic) in &self.package.topics {
            let suppressed = topic
                .inheritance
                .iter()
                .filter_map(|request| {
                    let target = match request {
                        InheritanceRequest::Inherit { target, .. }
                        | InheritanceRequest::InheritParams { target, .. }
                        | InheritanceRequest::InheritSection { target, .. } => target,
                    };
                    if !matches!(target, InheritTarget::Suppress(_)) {
                        return None;
                    }
                    Some(match request {
                        InheritanceRequest::Inherit { fields, .. } => requested_fields(fields),
                        InheritanceRequest::InheritParams { .. } => vec![InheritField::Params],
                        InheritanceRequest::InheritSection { .. } => vec![InheritField::Sections],
                    })
                })
                .flatten()
                .collect::<BTreeSet<_>>();
            let requests = topic
                .inheritance
                .iter()
                .cloned()
                .map(|request| {
                    let requested = match &request {
                        InheritanceRequest::Inherit { fields, .. } => requested_fields(fields),
                        InheritanceRequest::InheritParams { .. } => vec![InheritField::Params],
                        InheritanceRequest::InheritSection { .. } => vec![InheritField::Sections],
                    };
                    let effective_fields = requested
                        .into_iter()
                        .filter(|field| !suppressed.contains(field))
                        .collect::<Vec<_>>();
                    let target = match &request {
                        InheritanceRequest::Inherit { target, .. }
                        | InheritanceRequest::InheritParams { target, .. }
                        | InheritanceRequest::InheritSection { target, .. } => {
                            self.prepare_target(target)
                        }
                    };
                    PreparedRequest {
                        request,
                        target,
                        effective_fields,
                    }
                })
                .collect::<Vec<_>>();
            let requests = deduplicate_prepared_requests(
                requests,
                &mut self.diagnostics,
                self.current_package,
                self.links,
                self.inline_r_session,
            );
            let requests = requests
                .into_iter()
                .filter(|request| {
                    !request.effective_fields.is_empty()
                        || matches!(request.target, PreparedTarget::Suppressed)
                })
                .collect();
            self.normalized.insert(key.clone(), requests);
        }
    }

    fn prepare_target(&self, target: &InheritTarget) -> PreparedTarget {
        let InheritTarget::Topic(topic) = target else {
            return PreparedTarget::Suppressed;
        };
        let value = topic.value.0.as_str();
        let separators = value.match_indices("::").count();
        if value.contains(':') && separators != 1 {
            return PreparedTarget::Invalid {
                span: topic.span,
                message: "inheritance target has malformed package qualification",
                target: value.to_owned(),
            };
        }
        if separators == 0 {
            return match self.local_key(value) {
                Ok(key) => PreparedTarget::Local(key),
                Err(LocalLookupError::Missing) => PreparedTarget::Invalid {
                    span: topic.span,
                    message: "inheritance target was not found",
                    target: value.to_owned(),
                },
                Err(LocalLookupError::Ambiguous) => PreparedTarget::Invalid {
                    span: topic.span,
                    message: "inheritance target alias is ambiguous",
                    target: value.to_owned(),
                },
            };
        }
        let (package, name) = value.split_once("::").expect("one separator was counted");
        if package.is_empty() || name.is_empty() || package.contains(':') || name.contains(':') {
            return PreparedTarget::Invalid {
                span: topic.span,
                message: "inheritance target has malformed package qualification",
                target: value.to_owned(),
            };
        }
        if self.current_package == Some(package) {
            match self.local_key(name) {
                Ok(key) => PreparedTarget::Local(key),
                Err(LocalLookupError::Missing) => PreparedTarget::Invalid {
                    span: topic.span,
                    message: "inheritance target was not found",
                    target: value.to_owned(),
                },
                Err(LocalLookupError::Ambiguous) => PreparedTarget::Invalid {
                    span: topic.span,
                    message: "inheritance target alias is ambiguous",
                    target: value.to_owned(),
                },
            }
        } else {
            PreparedTarget::External {
                package: package.to_owned(),
                topic: name.to_owned(),
            }
        }
    }

    fn local_key(&self, requested: &str) -> Result<TopicKey, LocalLookupError> {
        lookup_local_topic(self.package, requested)
    }

    pub(super) fn graph(&self) -> GraphAnalysis<TopicKey> {
        let mut edges = BTreeMap::new();
        for key in self.package.topics.keys() {
            let donors = self
                .normalized
                .get(key)
                .into_iter()
                .flatten()
                .filter_map(|request| match &request.target {
                    PreparedTarget::Local(target)
                        if !request.effective_fields.is_empty()
                            && self.package.topics.contains_key(target) =>
                    {
                        Some(target.clone())
                    }
                    _ => None,
                })
                .collect();
            edges.insert(key.clone(), donors);
        }
        analyze_inheritance_graph(&edges)
    }

    fn emit_cycles(&mut self, nodes: &[TopicKey], cycle: Option<&CycleReport<TopicKey>>) {
        let Some(cycle) = cycle else { return };
        let path = cycle
            .path
            .iter()
            .map(TopicKey::as_str)
            .collect::<Vec<_>>()
            .join(" -> ");
        let mut labels = Vec::new();
        let mut primary = None;
        for node in nodes {
            for request in self.normalized.get(node).into_iter().flatten() {
                let PreparedTarget::Local(target) = &request.target else {
                    continue;
                };
                if nodes.contains(target) {
                    let origin = request_origin(&request.request);
                    let span = origin_span(&origin);
                    if primary.is_none() {
                        primary = Some(span);
                    } else {
                        labels.push(Label::new(span, "cycle edge"));
                    }
                }
            }
        }
        let Some(primary) = primary else { return };
        self.diagnostics.push(
            Diagnostic::new(
                Severity::Error,
                DiagnosticCode::InheritCycle,
                format!("inheritance cycle: {path}"),
                Label::new(primary, "cycle edge"),
            )
            .with_secondaries(labels),
        );
    }

    fn resolve_local(&mut self, key: &TopicKey) {
        if self.memo.contains_key(key) {
            return;
        }
        let raw = self
            .package
            .topics
            .get(key)
            .expect("resolution keys come from package topics")
            .clone();
        let prepared = self.normalized.get(key).cloned().unwrap_or_default();
        let donors = prepared
            .iter()
            .map(|request| self.donor(request))
            .collect::<Vec<_>>();
        let mut resolved = self.local_only_topic(key, &raw);
        let suppressed = prepared
            .iter()
            .filter(|request| matches!(request.target, PreparedTarget::Suppressed))
            .flat_map(|request| match &request.request {
                InheritanceRequest::Inherit { fields, .. } => requested_fields(fields),
                InheritanceRequest::InheritParams { .. } => vec![InheritField::Params],
                InheritanceRequest::InheritSection { .. } => vec![InheritField::Sections],
            })
            .collect::<BTreeSet<_>>();

        for field in [
            InheritField::Title,
            InheritField::Description,
            InheritField::Details,
            InheritField::Return,
            InheritField::SeeAlso,
            InheritField::References,
            InheritField::Examples,
            InheritField::Author,
            InheritField::Source,
            InheritField::Note,
            InheritField::Format,
        ] {
            if suppressed.contains(&field) {
                continue;
            }
            for (index, donor) in donors.iter().enumerate() {
                if !request_selects_field(&prepared[index], field) {
                    continue;
                }
                let Some(donor) = donor else { continue };
                merge_single_field(
                    &mut resolved,
                    donor,
                    field,
                    &request_origin(&prepared[index].request),
                );
            }
        }
        for (index, donor) in donors.iter().enumerate() {
            if !request_selects_field(&prepared[index], InheritField::Sections) {
                continue;
            }
            if suppressed.contains(&InheritField::Sections) {
                continue;
            }
            if let Some(donor) = donor {
                if matches!(
                    prepared[index].request,
                    InheritanceRequest::InheritSection { .. }
                ) {
                    continue;
                }
                merge_sections(
                    &mut resolved.sections,
                    &donor.sections,
                    &request_origin(&prepared[index].request),
                    self.current_package,
                    self.links,
                    Some(self.inline_r_session),
                );
            }
        }
        for (index, donor) in donors.iter().enumerate() {
            let InheritanceRequest::InheritSection { title, .. } = &prepared[index].request else {
                continue;
            };
            let Some(donor) = donor else { continue };
            if suppressed.contains(&InheritField::Sections) {
                continue;
            }
            merge_inherited_section(
                &mut resolved.sections,
                &donor.sections,
                &title.value,
                title.span,
                &request_origin(&prepared[index].request),
                self.current_package,
                self.links,
                Some(self.inline_r_session),
                &mut self.diagnostics,
            );
        }
        if !suppressed.contains(&InheritField::Params) {
            merge_params(
                &mut resolved,
                &raw,
                &prepared,
                &donors,
                &mut self.diagnostics,
            );
        } else {
            emit_missing_params(&raw, &resolved.params, &mut self.diagnostics);
        }
        self.memo.insert(key.clone(), resolved);
    }

    fn donor(&mut self, request: &PreparedRequest) -> Option<InheritableTopic> {
        let origin = request_origin(&request.request);
        match &request.target {
            PreparedTarget::Suppressed => None,
            PreparedTarget::Invalid { span, message, .. } => {
                self.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    DiagnosticCode::UnresolvedInherit,
                    *message,
                    Label::new(*span, "malformed inheritance target"),
                ));
                None
            }
            PreparedTarget::Local(key) => match self.memo.get(key) {
                Some(topic) => Some(topic_as_inheritable(key, topic)),
                None => {
                    self.diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        DiagnosticCode::UnresolvedInherit,
                        format!("local inheritance topic `{}` was not found", key.as_str()),
                        Label::new(origin_span(&origin), "unresolved local inheritance target"),
                    ));
                    None
                }
            },
            PreparedTarget::External { package, topic } => {
                let identity = DocumentationIdentity::External {
                    package: package.clone(),
                    topic: topic.clone(),
                };
                if self.options.external == ExternalInheritancePolicy::Off {
                    let reason = match self.options.external_source {
                        ExternalPolicySource::Explicit => "disabled by request",
                        ExternalPolicySource::NoConfiguredLibrary => {
                            "not attempted because no library path was configured"
                        }
                    };
                    self.diagnostics.push(Diagnostic::new(
                        Severity::Warning,
                        DiagnosticCode::ExternalInheritanceDisabled,
                        format!("external inheritance for `{package}::{topic}` was {reason}"),
                        Label::new(origin_span(&origin), "external inheritance skipped"),
                    ));
                    return None;
                }
                let lookup = self
                    .external_memo
                    .get(&identity)
                    .cloned()
                    .unwrap_or_else(|| {
                        let result = self.provider.get_topic(&TopicRequest::External {
                            package: package.clone(),
                            topic: TopicRef(topic.clone()),
                        });
                        self.external_memo.insert(identity.clone(), result.clone());
                        result
                    });
                match lookup {
                    Ok(Some(topic)) => Some(topic),
                    Ok(None) | Err(_) => {
                        let severity = match self.options.external {
                            ExternalInheritancePolicy::BestEffort => Severity::Warning,
                            ExternalInheritancePolicy::Strict => Severity::Error,
                            ExternalInheritancePolicy::Off => unreachable!(),
                        };
                        let message = match &lookup {
                            Ok(None) => format!(
                                "external inheritance topic `{package}::{topic}` was not found"
                            ),
                            Ok(Some(_)) => unreachable!(),
                            Err(error) => format!(
                                "could not load external inheritance topic `{package}::{topic}`: {}",
                                error.detail
                            ),
                        };
                        self.diagnostics.push(
                            Diagnostic::new(
                                severity,
                                DiagnosticCode::UnresolvedInherit,
                                message,
                                Label::new(origin_span(&origin), "external inheritance target"),
                            )
                            .with_context("package", package.clone())
                            .with_context("topic", topic.clone()),
                        );
                        None
                    }
                }
            }
        }
    }

    /// Projects a local topic and resolves section identity in the same
    /// context used by inherited selectors and final Rd lowering.
    fn local_only_topic(
        &mut self,
        key: &TopicKey,
        topic: &crate::model::RdTopic,
    ) -> ResolvedRdTopic {
        let mut resolved = local_only_topic(key, topic);
        let mut retained = Vec::with_capacity(resolved.sections.len());
        let mut titles = BTreeMap::new();
        for section in resolved.sections {
            let title_key = super::merge::section_key(
                &section,
                self.current_package,
                self.links,
                Some(self.inline_r_session),
            );
            if let Some(previous) = titles.get(&title_key).cloned() {
                let later_span = local_title_span(&section.title)
                    .expect("local section titles have local provenance");
                let first_span = local_title_span(&previous)
                    .expect("local section titles have local provenance");
                self.diagnostics.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagnosticCode::ConflictingSectionTitle,
                        format!(
                            "topic `{}` has more than one section with the same semantic title",
                            key.as_str()
                        ),
                        Label::new(later_span, "conflicting section title"),
                    )
                    .with_secondary(Label::new(first_span, "first section with this title")),
                );
                continue;
            }
            titles.insert(title_key, section.title.clone());
            retained.push(section);
        }
        resolved.sections = retained;
        resolved
    }
}

fn local_title_span(content: &super::types::ResolvedContent) -> Option<Span> {
    match &content.provenance.source {
        super::types::DocumentationOrigin::Local(origin) => Some(origin_span(origin)),
        super::types::DocumentationOrigin::External { .. } => None,
    }
}
