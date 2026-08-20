use std::collections::BTreeMap;

use rd_ast::RdNode;

use crate::model::{Alias, InheritanceRequest, PackageMetadataDiagnosticState, TopicKey};
use crate::source::Span;
use crate::tags::{
    DocName, ExamplesContent, InheritField, MarkdownText, ParamName, RCodeText, TagOrigin, TagValue,
};

use super::provider::DocumentationIdentity;

/// Content which can cross the local/external documentation boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum InheritableContent {
    /// Local prose which still needs Markdown conversion.
    Markdown(MarkdownText),
    /// Local R source which must be emitted without evaluation.
    RCode(RCodeText),
    /// One typed examples contribution, including static conditional examples.
    Examples(ExamplesContent),
    /// Already-lowered external Rd nodes.
    Rd(Vec<RdNode>),
}

/// Logical origin of inherited content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentationOrigin {
    /// A tag in the package currently being documented.
    Local(TagOrigin),
    /// A component from an installed or otherwise external topic.
    External {
        /// The package containing the donor.
        package: String,
        /// The canonical donor topic.
        topic: String,
        /// The component containing the content.
        component: InheritField,
    },
}

/// Causality and authorship for one inherited content value.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritanceTrace {
    /// The donor tag or external logical component that authored the content.
    pub source: DocumentationOrigin,
    /// Requests, in order, which caused the content to be copied.
    pub requests: Vec<TagOrigin>,
}

/// Content together with its donor and request provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedContent {
    /// The content value at the Rd boundary.
    pub value: InheritableContent,
    /// Its authorship and inheritance chain.
    pub provenance: InheritanceTrace,
}

impl ResolvedContent {
    pub(super) fn local_markdown(value: &TagValue<MarkdownText>) -> Self {
        Self {
            value: InheritableContent::Markdown(value.value.clone()),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::Local(value.origin.clone()),
                requests: Vec::new(),
            },
        }
    }

    pub(super) fn local_examples(value: &TagValue<crate::tags::ExamplesContent>) -> Self {
        Self {
            value: InheritableContent::Examples(value.value.clone()),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::Local(value.origin.clone()),
                requests: Vec::new(),
            },
        }
    }

    pub(super) fn through_request(&self, request: &TagOrigin) -> Self {
        let mut value = self.clone();
        value.provenance.requests.push(request.clone());
        value
    }

    pub(super) fn through_requests(&self, requests: &[TagOrigin]) -> Self {
        let mut value = self.clone();
        value.provenance.requests.extend(requests.iter().cloned());
        value
    }
}

/// The single-value fields that can be inherited independently.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InheritableFields {
    /// `@title`.
    pub title: Option<ResolvedContent>,
    /// `@description`.
    pub description: Option<ResolvedContent>,
    /// `@details`.
    pub details: Option<ResolvedContent>,
    /// `@return`.
    pub return_value: Option<ResolvedContent>,
    /// `@seealso`.
    pub see_also: Option<ResolvedContent>,
    /// `@references`.
    pub references: Option<ResolvedContent>,
    /// The one `@examples` or `@examplesIf` contribution.
    pub examples: Option<ResolvedContent>,
    /// `@author`.
    pub author: Option<ResolvedContent>,
    /// `@source`.
    pub source: Option<ResolvedContent>,
    /// `@note`.
    pub note: Option<ResolvedContent>,
    /// `@format`.
    pub format: Option<ResolvedContent>,
}

/// A parameter documentation group. Names remain grouped because inheritance
/// copies a multi-name `@param` entry all-or-nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritableParamGroup {
    /// Names written by the donor's one parameter tag.
    pub names: Vec<ParamName>,
    /// Display label written by the donor, when preserving its Rd markup is
    /// safe after inheritance matching.
    pub label: InheritableParamLabel,
    /// The group's description and provenance.
    pub description: ResolvedContent,
}

/// The display label for one inherited parameter group.
#[derive(Debug, Clone, PartialEq)]
pub enum InheritableParamLabel {
    /// Render the semantic names as plain text.
    Generated,
    /// Render the original Rd nodes from the donor's `\\item` label.
    Rd(Vec<RdNode>),
}

/// A named section with independently traceable title and body content.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritableSection {
    /// The semantic section title.
    pub title: ResolvedContent,
    /// The section body.
    pub body: ResolvedContent,
}

/// A topic projected into the shared local/external inheritance boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritableTopic {
    /// Canonical documentation identity when known by the provider.
    pub identity: DocumentationIdentity,
    /// Grouped parameter documentation.
    pub params: Vec<InheritableParamGroup>,
    /// Single-value fields.
    pub fields: InheritableFields,
    /// Named sections.
    pub sections: Vec<InheritableSection>,
    /// Requests are present for local projections and absent after resolution.
    pub(crate) requests: Vec<InheritanceRequest>,
}

/// A resolved topic has the same output metadata as `RdTopic`, but all content
/// fields use resolved values. This factoring avoids duplicating identity,
/// aliases, usages, and formal facts while making pending requests impossible.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRdTopic {
    /// Topic identity and output metadata.
    pub name: DocName,
    /// Topic kind used by the Rd renderer.
    pub kind: crate::model::RdTopicKind,
    /// Source span that established the package/data kind, when applicable.
    pub kind_origin: Option<Span>,
    /// Whether the topic had incompatible package/data contributions.
    pub kind_conflict_reported: bool,
    /// Source blocks contributing the topic.
    pub blocks: Vec<crate::model::BlockRef>,
    /// Topic aliases.
    pub aliases: Vec<Alias>,
    /// Topic keywords.
    pub keywords: Vec<crate::tags::Keyword>,
    /// Resolved title.
    pub title: Option<ResolvedContent>,
    /// Resolved description.
    pub description: Option<ResolvedContent>,
    /// Whether `@description NULL` in any block contributing to a package
    /// topic suppresses DESCRIPTION fallback and title-based description
    /// regeneration.
    pub description_suppressed: bool,
    /// Resolved details.
    pub details: Option<ResolvedContent>,
    /// Resolved return value.
    pub return_value: Option<ResolvedContent>,
    /// Usages retained from the model layer.
    pub usages: Vec<crate::model::UsageContribution>,
    /// Formal facts retained from the model layer.
    pub formals: Vec<crate::model::FormalContribution>,
    /// Resolved parameter groups.
    pub params: Vec<InheritableParamGroup>,
    /// Resolved named sections.
    pub sections: Vec<InheritableSection>,
    /// Remaining resolved single-value fields.
    pub see_also: Option<ResolvedContent>,
    /// Remaining resolved single-value fields.
    pub references: Option<ResolvedContent>,
    /// Remaining resolved single-value fields.
    pub note: Option<ResolvedContent>,
    /// Remaining resolved single-value fields.
    pub format: Option<ResolvedContent>,
    /// The first data-object contribution that omitted `@format`.
    pub missing_data_format_span: Option<Span>,
    /// Remaining resolved single-value fields.
    pub source: Option<ResolvedContent>,
    /// Remaining resolved single-value fields.
    pub author: Option<ResolvedContent>,
    /// The resolved examples contribution, when supplied.
    pub examples: Option<ResolvedContent>,
    /// DESCRIPTION-derived package author content.
    pub package_author: Option<crate::model::PackageAuthor>,
    /// DESCRIPTION-derived package see-also content.
    pub package_see_also: Option<crate::model::PackageSeeAlso>,
    /// DESCRIPTION diagnostics retained for final resolved-field checks.
    pub package_metadata_diagnostics: Option<PackageMetadataDiagnosticState>,
}

/// Package model after all inheritance requests have been consumed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedPackageModel {
    /// Topics with no pending inheritance requests.
    pub topics: BTreeMap<TopicKey, ResolvedRdTopic>,
    /// NAMESPACE requests are unchanged by inheritance resolution.
    pub namespace: Vec<crate::model::NamespaceRequest>,
}
