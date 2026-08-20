use std::collections::BTreeSet;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::markdown_conversion::section_key::SectionTitleKey;
use crate::markdown_conversion::{markdown_section_key, rd_section_key};
use crate::model::{FormalNames, InheritanceRequest, ParamDescription, RdTopic, TopicKey};
use crate::source::Span;
use crate::tags::{InheritField, InheritFields, MarkdownText, ParamName, TagOrigin};

use super::provider::DocumentationIdentity;
use super::resolver::PreparedRequest;
use super::selection::evaluate_selection;
use super::types::{
    DocumentationOrigin, InheritableContent, InheritableFields, InheritableParamGroup,
    InheritableParamLabel, InheritableSection, InheritableTopic, InheritanceTrace, ResolvedContent,
    ResolvedRdTopic,
};

pub(super) fn project_local_topic(key: &TopicKey, topic: &RdTopic) -> InheritableTopic {
    InheritableTopic {
        identity: DocumentationIdentity::Local(key.clone()),
        params: local_params(&topic.params),
        fields: local_fields(topic),
        sections: topic
            .sections
            .iter()
            .map(|section| InheritableSection {
                title: local_content(&section.title, &section.origin),
                body: local_content(&section.body, &section.origin),
            })
            .collect(),
        requests: topic.inheritance.clone(),
    }
}

pub(super) fn local_only_topic(key: &TopicKey, topic: &RdTopic) -> ResolvedRdTopic {
    let view = project_local_topic(key, topic);
    resolved_topic(topic, view)
}

fn resolved_topic(topic: &RdTopic, view: InheritableTopic) -> ResolvedRdTopic {
    ResolvedRdTopic {
        name: topic.name.clone(),
        kind: topic.kind,
        kind_origin: topic.kind_origin.map(|origin| origin.span),
        kind_conflict_reported: topic.kind_conflict_reported,
        blocks: topic.blocks.clone(),
        aliases: topic.aliases.clone(),
        keywords: topic.keywords.clone(),
        title: view.fields.title,
        description: view.fields.description,
        description_suppressed: topic.description_suppressed,
        details: view.fields.details,
        return_value: view.fields.return_value,
        usages: topic.usages.clone(),
        formals: topic.formals.clone(),
        params: view.params,
        sections: view.sections,
        see_also: view.fields.see_also,
        references: view.fields.references,
        note: view.fields.note,
        format: view.fields.format,
        missing_data_format_span: topic.missing_data_format_span,
        source: view.fields.source,
        author: view.fields.author,
        examples: view.fields.examples,
        package_author: topic.package_author.clone(),
        package_see_also: topic.package_see_also.clone(),
        package_metadata_diagnostics: topic.package_metadata_diagnostics.clone(),
    }
}

fn local_content(value: &MarkdownText, origin: &TagOrigin) -> ResolvedContent {
    ResolvedContent {
        value: InheritableContent::Markdown(value.clone()),
        provenance: InheritanceTrace {
            source: DocumentationOrigin::Local(origin.clone()),
            requests: Vec::new(),
        },
    }
}

fn local_fields(topic: &RdTopic) -> InheritableFields {
    InheritableFields {
        title: topic.title.as_ref().map(ResolvedContent::local_markdown),
        description: topic
            .description
            .as_ref()
            .map(ResolvedContent::local_markdown),
        details: topic.details.as_ref().map(ResolvedContent::local_markdown),
        return_value: topic
            .return_value
            .as_ref()
            .map(ResolvedContent::local_markdown),
        see_also: topic.see_also.as_ref().map(ResolvedContent::local_markdown),
        references: topic
            .references
            .as_ref()
            .map(ResolvedContent::local_markdown),
        examples: topic.examples.as_ref().map(ResolvedContent::local_examples),
        author: topic.author.as_ref().map(ResolvedContent::local_markdown),
        source: topic.source.as_ref().map(ResolvedContent::local_markdown),
        note: topic.note.as_ref().map(ResolvedContent::local_markdown),
        format: topic.format.as_ref().map(ResolvedContent::local_markdown),
    }
}

fn local_params(params: &[ParamDescription]) -> Vec<InheritableParamGroup> {
    let mut result: Vec<InheritableParamGroup> = Vec::new();
    for parameter in params {
        if let Some(previous) = result.last_mut()
            && previous.description.provenance.source
                == DocumentationOrigin::Local(parameter.origin.clone())
            && matches!(&previous.description.value, InheritableContent::Markdown(value) if *value == parameter.description)
        {
            previous.names.push(parameter.name.clone());
            continue;
        }
        result.push(InheritableParamGroup {
            names: vec![parameter.name.clone()],
            label: InheritableParamLabel::Generated,
            description: local_content(&parameter.description, &parameter.origin),
        });
    }
    result
}

pub(super) fn topic_as_inheritable(key: &TopicKey, topic: &ResolvedRdTopic) -> InheritableTopic {
    InheritableTopic {
        identity: DocumentationIdentity::Local(key.clone()),
        params: topic.params.clone(),
        fields: InheritableFields {
            title: topic.title.clone(),
            description: topic.description.clone(),
            details: topic.details.clone(),
            return_value: topic.return_value.clone(),
            see_also: topic.see_also.clone(),
            references: topic.references.clone(),
            examples: topic.examples.clone(),
            author: topic.author.clone(),
            source: topic.source.clone(),
            note: topic.note.clone(),
            format: topic.format.clone(),
        },
        sections: topic.sections.clone(),
        requests: Vec::new(),
    }
}

pub(super) fn request_origin(request: &InheritanceRequest) -> TagOrigin {
    match request {
        InheritanceRequest::Inherit { origin, .. }
        | InheritanceRequest::InheritParams { origin, .. }
        | InheritanceRequest::InheritSection { origin, .. } => origin.clone(),
    }
}

pub(super) fn origin_span(origin: &TagOrigin) -> Span {
    match origin {
        TagOrigin::Explicit { full_span, .. } => *full_span,
        TagOrigin::Implicit { intro_span } => *intro_span,
    }
}

pub(super) fn requested_fields(fields: &InheritFields) -> Vec<InheritField> {
    match fields {
        InheritFields::All { .. } => vec![
            InheritField::Params,
            InheritField::Return,
            InheritField::Title,
            InheritField::Description,
            InheritField::Details,
            InheritField::SeeAlso,
            InheritField::Sections,
            InheritField::References,
            InheritField::Examples,
            InheritField::Author,
            InheritField::Source,
            InheritField::Note,
            InheritField::Format,
        ],
        InheritFields::Selected(fields) => fields.iter().map(|field| field.value).collect(),
    }
}

pub(super) fn request_selects_field(request: &PreparedRequest, field: InheritField) -> bool {
    request.effective_fields.contains(&field)
}

pub(super) fn merge_single_field(
    topic: &mut ResolvedRdTopic,
    donor: &InheritableTopic,
    field: InheritField,
    request: &TagOrigin,
) {
    let destination = match field {
        InheritField::Title => &mut topic.title,
        InheritField::Description => &mut topic.description,
        InheritField::Details => &mut topic.details,
        InheritField::Return => &mut topic.return_value,
        InheritField::SeeAlso => &mut topic.see_also,
        InheritField::References => &mut topic.references,
        InheritField::Examples => {
            if topic.examples.is_none() {
                topic.examples = donor
                    .fields
                    .examples
                    .as_ref()
                    .map(|content| content.through_request(request));
            }
            return;
        }
        InheritField::Author => &mut topic.author,
        InheritField::Source => &mut topic.source,
        InheritField::Note => &mut topic.note,
        InheritField::Format => &mut topic.format,
        InheritField::Params | InheritField::Sections => return,
    };
    if destination.is_none() {
        let source = match field {
            InheritField::Title => donor.fields.title.as_ref(),
            InheritField::Description => donor.fields.description.as_ref(),
            InheritField::Details => donor.fields.details.as_ref(),
            InheritField::Return => donor.fields.return_value.as_ref(),
            InheritField::SeeAlso => donor.fields.see_also.as_ref(),
            InheritField::References => donor.fields.references.as_ref(),
            InheritField::Examples => None,
            InheritField::Author => donor.fields.author.as_ref(),
            InheritField::Source => donor.fields.source.as_ref(),
            InheritField::Note => donor.fields.note.as_ref(),
            InheritField::Format => donor.fields.format.as_ref(),
            InheritField::Params | InheritField::Sections => None,
        };
        *destination = source.map(|content| content.through_request(request));
    }
}

pub(super) fn merge_sections(
    destination: &mut Vec<InheritableSection>,
    donor: &[InheritableSection],
    request: &TagOrigin,
    current_package: Option<&str>,
    links: &dyn crate::rd::RdLinkResolver,
    inline_r_session: Option<&crate::inline_r::InlineRSession<'_>>,
) {
    let mut titles = destination
        .iter()
        .map(|section| section_key(section, current_package, links, inline_r_session))
        .collect::<BTreeSet<_>>();
    for section in donor {
        if titles.insert(section_key(
            section,
            current_package,
            links,
            inline_r_session,
        )) {
            destination.push(InheritableSection {
                title: section.title.through_request(request),
                body: section.body.through_request(request),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn merge_inherited_section(
    destination: &mut Vec<InheritableSection>,
    donor: &[InheritableSection],
    title: &MarkdownText,
    title_span: Span,
    request: &TagOrigin,
    current_package: Option<&str>,
    links: &dyn crate::rd::RdLinkResolver,
    inline_r_session: Option<&crate::inline_r::InlineRSession<'_>>,
    diagnostics: &mut Diagnostics,
) {
    let requested_key = markdown_section_key(title, current_package, links, inline_r_session);
    let matches = donor
        .iter()
        .filter(|section| {
            section_key(section, current_package, links, inline_r_session) == requested_key
        })
        .collect::<Vec<_>>();
    let label = Label::new(title_span, "requested section title");
    match matches.as_slice() {
        [] => diagnostics.push(Diagnostic::new(
            Severity::Error,
            DiagnosticCode::MissingInheritedSection,
            format!("donor has no section named `{}`", title.as_str()),
            label,
        )),
        [section] => {
            if destination.iter().any(|section| {
                section_key(section, current_package, links, inline_r_session) == requested_key
            }) {
                return;
            }
            destination.push(InheritableSection {
                title: section.title.through_request(request),
                body: section.body.through_request(request),
            });
        }
        _ => diagnostics.push(Diagnostic::new(
            Severity::Error,
            DiagnosticCode::AmbiguousInheritedSection,
            format!("donor has more than one section named `{}`", title.as_str()),
            label,
        )),
    }
}

pub(super) fn section_key(
    section: &InheritableSection,
    current_package: Option<&str>,
    links: &dyn crate::rd::RdLinkResolver,
    inline_r_session: Option<&crate::inline_r::InlineRSession<'_>>,
) -> SectionTitleKey {
    match &section.title.value {
        InheritableContent::Markdown(value) => {
            markdown_section_key(value, current_package, links, inline_r_session)
        }
        // Section titles are Markdown locally and Rd externally in practice.
        // Keep this defensive arm so the match remains total if an RCode
        // section title is supplied.
        InheritableContent::RCode(value) => SectionTitleKey::from_text(value.as_str()),
        InheritableContent::Examples(value) => match value {
            crate::tags::ExamplesContent::Ordinary(value) => {
                SectionTitleKey::from_text(value.as_str())
            }
            crate::tags::ExamplesContent::Conditional(value) => {
                SectionTitleKey::from_text(value.body.as_str())
            }
        },
        InheritableContent::Rd(nodes) => rd_section_key(nodes),
    }
}

pub(super) fn merge_params(
    topic: &mut ResolvedRdTopic,
    raw: &RdTopic,
    requests: &[PreparedRequest],
    donors: &[Option<InheritableTopic>],
    diagnostics: &mut Diagnostics,
) {
    let formals = match raw.inheritance_formal_names() {
        FormalNames::Known(names) => names,
        FormalNames::NotFunction
        | FormalNames::Unknown { .. }
        | FormalNames::Undecodable { .. } => Vec::new(),
    };
    let mut missing = formals
        .iter()
        .map(|formal| formal.name.clone())
        .collect::<Vec<_>>();
    for group in &topic.params {
        for name in &group.names {
            missing.retain(|candidate| candidate != name);
        }
    }
    let mut aggregated: Vec<(InheritableTopic, BTreeSet<String>, Vec<ParamSelection>)> = Vec::new();
    for (index, donor) in donors.iter().enumerate() {
        let Some(donor) = donor else { continue };
        if !request_selects_field(&requests[index], InheritField::Params) {
            continue;
        }
        let selection = match &requests[index].request {
            InheritanceRequest::InheritParams { selection, .. } => selection.as_ref(),
            InheritanceRequest::Inherit { .. } | InheritanceRequest::InheritSection { .. } => None,
        };
        let domain = donor
            .params
            .iter()
            .flat_map(|group| group.names.iter())
            .filter(|name| name.0 != "...")
            .cloned()
            .collect::<Vec<_>>();
        let selected = match selection {
            Some(selection) => match evaluate_selection(&domain, selection) {
                Ok(selected) => selected,
                Err(error) => {
                    // Compatibility choice: roxygen2 warns and copies no
                    // parameters here; mini-roxygen treats a malformed
                    // selection as an error so CI cannot silently lose docs.
                    diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        DiagnosticCode::InvalidSelection,
                        format!("invalid inherited-parameter selection: {:?}", error.kind),
                        Label::new(error.span, "invalid parameter selection"),
                    ));
                    continue;
                }
            },
            None => domain.clone(),
        };
        let selection_request = ParamSelection {
            names: selected.iter().map(|name| name.0.clone()).collect(),
            unfiltered: selection.is_none(),
            origin: request_origin(&requests[index].request),
        };
        let Some(existing) = aggregated
            .iter_mut()
            .find(|(existing, _, _)| existing.identity == donor.identity)
        else {
            aggregated.push((
                donor.clone(),
                selection_request.names.clone(),
                vec![selection_request],
            ));
            continue;
        };
        existing.1.extend(selection_request.names.iter().cloned());
        existing.2.push(selection_request);
    }
    for (donor, selected_names, selection_requests) in aggregated {
        for group in &donor.params {
            let has_unfiltered_request =
                selection_requests.iter().any(|request| request.unfiltered);
            let eligible = has_unfiltered_request
                || group
                    .names
                    .iter()
                    .all(|name| selected_names.contains(&name.0));
            if !eligible {
                continue;
            }
            let mut covered = Vec::new();
            let mut complete = true;
            for name in &group.names {
                let matches = matching_names(name, &missing);
                if matches.is_empty() {
                    complete = false;
                    break;
                }
                covered.extend(matches);
            }
            if !complete {
                continue;
            }
            let preserves_label = covered == group.names;
            covered.sort_by_key(|name| {
                formals
                    .iter()
                    .position(|formal| &formal.name == name)
                    .unwrap_or(usize::MAX)
            });
            covered.dedup();
            let preserves_label = preserves_label && covered == group.names;
            if covered.is_empty() && group.names.iter().any(|name| name.0 == "...") {
                continue;
            }
            for name in &covered {
                missing.retain(|candidate| candidate != name);
            }
            let request_origins = selection_requests
                .iter()
                .filter(|request| {
                    request.unfiltered
                        || group
                            .names
                            .iter()
                            .any(|name| request.names.contains(&name.0))
                })
                .map(|request| request.origin.clone())
                .collect::<Vec<_>>();
            topic.params.push(InheritableParamGroup {
                names: covered,
                label: if preserves_label {
                    group.label.clone()
                } else {
                    InheritableParamLabel::Generated
                },
                description: group.description.through_requests(&request_origins),
            });
        }
    }
    let mut formal_groups = Vec::new();
    let mut nonformal_groups = Vec::new();
    for group in topic.params.drain(..) {
        if let Some(index) = group
            .names
            .iter()
            .filter_map(|name| formals.iter().position(|formal| &formal.name == name))
            .min()
        {
            formal_groups.push((index, group));
        } else {
            nonformal_groups.push(group);
        }
    }
    formal_groups.sort_by_key(|(index, _)| *index);
    topic.params = formal_groups
        .into_iter()
        .map(|(_, group)| group)
        .chain(nonformal_groups)
        .collect();
    for formal in formals {
        if missing.contains(&formal.name) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::MissingParam.default_severity(),
                DiagnosticCode::MissingParam,
                format!("parameter `{}` is not documented", formal.name.0),
                Label::new(formal.span, "missing parameter documentation"),
            ));
        }
    }
}

#[derive(Debug, Clone)]
struct ParamSelection {
    names: BTreeSet<String>,
    unfiltered: bool,
    origin: TagOrigin,
}

pub(super) fn emit_missing_params(
    raw: &RdTopic,
    params: &[InheritableParamGroup],
    diagnostics: &mut Diagnostics,
) {
    let FormalNames::Known(formals) = raw.inheritance_formal_names() else {
        return;
    };
    let mut missing = formals
        .iter()
        .map(|formal| formal.name.clone())
        .collect::<Vec<_>>();
    for group in params {
        for name in &group.names {
            missing.retain(|candidate| candidate != name);
        }
    }
    for formal in formals {
        if missing.contains(&formal.name) {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::MissingParam.default_severity(),
                DiagnosticCode::MissingParam,
                format!("parameter `{}` is not documented", formal.name.0),
                Label::new(formal.span, "missing parameter documentation"),
            ));
        }
    }
}

fn matching_names(name: &ParamName, candidates: &[ParamName]) -> Vec<ParamName> {
    let toggled = if let Some(stripped) = name.0.strip_prefix('.') {
        ParamName(stripped.to_owned())
    } else {
        ParamName(format!(".{}", name.0))
    };
    let mut matches = candidates
        .iter()
        .filter(|candidate| candidate == &name)
        .cloned()
        .collect::<Vec<_>>();
    matches.extend(
        candidates
            .iter()
            .filter(|candidate| candidate == &&toggled)
            .cloned()
            .collect::<Vec<_>>(),
    );
    matches
}
