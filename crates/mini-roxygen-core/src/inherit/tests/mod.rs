use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use rd_ast::RdNode;

use crate::diagnostic::DiagnosticCode;
use crate::model::build_package_model;
use crate::model::test_support::{blocks as source_blocks, model, model_with_tag_diagnostics};
use crate::rd::{RdLinkResolution, RdLinkResolver};
use crate::source::SourceMap;

use super::resolver;
use super::{
    DocumentationError, DocumentationErrorKind, DocumentationIdentity, DocumentationOrigin,
    DocumentationProvider, ExternalInheritancePolicy, ExternalPolicySource, InheritableContent,
    InheritableFields, InheritableParamGroup, InheritableParamLabel, InheritableSection,
    InheritableTopic, InheritanceOptions, InheritanceTrace, ResolvedContent, ResolvedPackageModel,
    TopicRequest, resolve_inheritance,
};

pub(super) struct EmptyProvider;

pub(super) struct NoLinks;

impl RdLinkResolver for NoLinks {
    fn resolve_unqualified(&self, _topic: &str) -> RdLinkResolution {
        RdLinkResolution::Unresolved
    }
}

pub(super) static NO_LINKS: NoLinks = NoLinks;

impl DocumentationProvider for EmptyProvider {
    fn get_topic(
        &self,
        _request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        Ok(None)
    }
}

pub(super) struct CountingProvider {
    calls: Cell<usize>,
    result: Result<Option<InheritableTopic>, DocumentationError>,
}

impl DocumentationProvider for CountingProvider {
    fn get_topic(
        &self,
        _request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        self.calls.set(self.calls.get() + 1);
        self.result.clone()
    }
}

pub(super) struct RdSectionsProvider {
    sections: BTreeMap<String, Vec<Vec<RdNode>>>,
}

impl DocumentationProvider for RdSectionsProvider {
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        let TopicRequest::External { topic, .. } = request else {
            return Ok(None);
        };
        let Some(titles) = self.sections.get(&topic.0) else {
            return Ok(None);
        };
        let mut donor = external_title(&topic.0);
        donor.sections = titles
            .iter()
            .enumerate()
            .map(|(index, title)| {
                let mut section = external_section(&topic.0, index);
                section.title.value = InheritableContent::Rd(title.clone());
                section
            })
            .collect();
        Ok(Some(donor))
    }
}

pub(super) fn param_names(topic: &super::ResolvedRdTopic) -> Vec<Vec<String>> {
    topic
        .params
        .iter()
        .map(|group| group.names.iter().map(|name| name.0.clone()).collect())
        .collect()
}

pub(super) fn external_section_count(
    source: &str,
    sections: BTreeMap<String, Vec<Vec<RdNode>>>,
) -> usize {
    let input = model(source);
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &RdSectionsProvider { sections },
        &options,
    );
    output.package.topics[&crate::model::TopicKey("target".into())]
        .sections
        .len()
}

pub(super) fn external_section(topic: &str, index: usize) -> InheritableSection {
    let mut donor = external_title(topic);
    let section_index = index.min(donor.sections.len() - 1);
    donor.sections.remove(section_index)
}

pub(super) fn external_title(topic: &str) -> InheritableTopic {
    InheritableTopic {
        identity: super::DocumentationIdentity::External {
            package: "pkg".to_owned(),
            topic: topic.to_owned(),
        },
        params: Vec::new(),
        fields: InheritableFields {
            title: Some(ResolvedContent {
                value: InheritableContent::Rd(vec![RdNode::Text(topic.to_owned())]),
                provenance: InheritanceTrace {
                    source: DocumentationOrigin::External {
                        package: "pkg".to_owned(),
                        topic: topic.to_owned(),
                        component: crate::tags::InheritField::Title,
                    },
                    requests: Vec::new(),
                },
            }),
            ..InheritableFields::default()
        },
        sections: vec![InheritableSection {
            title: ResolvedContent {
                value: InheritableContent::Rd(vec![RdNode::Text(topic.to_owned())]),
                provenance: InheritanceTrace {
                    source: DocumentationOrigin::External {
                        package: "pkg".to_owned(),
                        topic: topic.to_owned(),
                        component: crate::tags::InheritField::Sections,
                    },
                    requests: Vec::new(),
                },
            },
            body: ResolvedContent {
                value: InheritableContent::Rd(vec![RdNode::Text("Body".to_owned())]),
                provenance: InheritanceTrace {
                    source: DocumentationOrigin::External {
                        package: "pkg".to_owned(),
                        topic: topic.to_owned(),
                        component: crate::tags::InheritField::Sections,
                    },
                    requests: Vec::new(),
                },
            },
        }],
        requests: Vec::new(),
    }
}

mod external;
mod local;
mod merge;
mod sections;
