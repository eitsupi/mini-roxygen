//! Public intermediate representation for package documentation topics.
//!
//! These source-backed types are kept apart from merge and resolution logic so
//! later layers can depend on stable model data without depending on how it was assembled.

use std::collections::BTreeMap;

use crate::arity_adapter::{AuthorsParseError, BlockId, RName};
use crate::diagnostic::Diagnostics;
use crate::r_parse::{BindingFact, BlockTarget};
use crate::s3_register::S3RegistrationFact;
use crate::source::{FileId, Span, Spanned};
use crate::tags::{
    DocName, ExamplesContent, InheritFields, InheritTarget, Keyword, MarkdownText, NamespaceTag,
    ParamName, ParsedTag, RCodeText, TagOrigin, TagValue,
};
use crate::usage::GeneratedUsage;

/// Identifies one documentation block without confusing file-local block IDs.
///
/// `BlockId` is intentionally only unique within one parsed file, so every
/// model lookup and every provenance record must carry the owning file too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockRef {
    /// The source file containing the block.
    pub file: FileId,
    /// The file-local block identity.
    pub block: BlockId,
}

/// A block after tags have been parsed but before topic merging.
///
/// Keeping the block span and syntactic target together lets this layer report
/// association failures precisely without re-reading parser or source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentedBlock {
    /// The stable identity of the documentation block.
    pub block: BlockRef,
    /// The source span of the documentation block.
    pub block_span: Span,
    /// The syntax-only object associated with the block.
    pub target: BlockTarget,
    /// The semantic tags belonging to the block.
    pub tags: Vec<ParsedTag>,
}

/// The result of building all topics and deferred namespace requests.
///
/// Diagnostics are returned alongside partial IR so callers can inspect or
/// render unaffected topics while still failing the overall generation when
/// an error was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOutput {
    /// The package-wide topic and namespace intermediate representation.
    pub package: PackageModel,
    /// Errors and recoverable issues found while merging blocks.
    pub diagnostics: Diagnostics,
}

/// Package-wide model data awaiting inheritance, Rd, and NAMESPACE lowering.
///
/// Ordered maps and first-seen vectors make output independent of hash table
/// iteration order and preserve the documented merge order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageModel {
    /// Topics keyed by their eventual Rd file identity.
    pub topics: BTreeMap<TopicKey, RdTopic>,
    /// NAMESPACE requests retained verbatim for the namespace layer.
    pub namespace: Vec<NamespaceRequest>,
    /// All top-level package bindings used by static namespace analysis.
    pub bindings: Vec<BindingFact>,
    /// Static S3 registrar facts collected from package source.
    pub registrations: Vec<S3RegistrationFact>,
    /// Whether DESCRIPTION contains any `Collate`-family directive.
    pub collate: bool,
}

/// The identity used to merge blocks into one Rd topic.
///
/// A newtype prevents accidentally passing a display name where a merge key
/// is required, while `Ord` gives the package model deterministic iteration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TopicKey(pub String);

impl TopicKey {
    /// Returns the textual topic key without exposing its representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One merged topic before it is converted to Rd nodes.
///
/// The fields intentionally retain typed tag values and usage provenance. In
/// particular, generated and explicit usage are not collapsed into strings,
/// because later output may need to distinguish their different guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdTopic {
    /// The Rd display name from the first contribution after ordering, using
    /// its explicit or implicit primary name. Merge, local-lookup, and output
    /// path identity are carried separately by `TopicKey`.
    pub name: DocName,
    /// Whether this is the package-level documentation topic.
    pub kind: RdTopicKind,
    /// The first package/data contribution that established a non-ordinary
    /// kind. This remains available even when aliases are suppressed.
    pub(crate) kind_origin: Option<TopicKindOrigin>,
    /// Whether the one diagnostic for a package/data kind conflict was emitted.
    pub(crate) kind_conflict_reported: bool,
    /// Documentation blocks contributing to this topic, in merge order.
    pub blocks: Vec<BlockRef>,
    /// Aliases in first-seen order, retaining the source span of each claim.
    pub aliases: Vec<Alias>,
    /// Keywords in first-seen order.
    pub keywords: Vec<Keyword>,
    /// The one title slot, when supplied.
    pub title: Option<TagValue<MarkdownText>>,
    /// The one description slot, when supplied.
    pub description: Option<TagValue<MarkdownText>>,
    /// Whether `@description NULL` in any block contributing to a package
    /// topic suppresses DESCRIPTION fallback and title-based description
    /// regeneration.
    pub description_suppressed: bool,
    /// The one details slot, when supplied.
    pub details: Option<TagValue<MarkdownText>>,
    /// The one return-description slot, when supplied.
    pub return_value: Option<TagValue<MarkdownText>>,
    /// The one ordinary or conditional examples slot, when supplied.
    pub examples: Option<TagValue<ExamplesContent>>,
    /// Usage and optional S3 method contributions in merge order.
    pub usages: Vec<UsageContribution>,
    /// Ordered formal-name facts contributed by documented blocks.
    pub formals: Vec<FormalContribution>,
    /// Parameter descriptions in contribution order; names remain available
    /// on each entry for conflict detection and later lookup.
    pub params: Vec<ParamDescription>,
    /// Named sections in contribution order; titles remain available on each
    /// entry for conflict detection and later lookup.
    pub sections: Vec<NamedSection>,
    /// The one see-also slot, when supplied.
    pub see_also: Option<TagValue<MarkdownText>>,
    /// The one references slot, when supplied.
    pub references: Option<TagValue<MarkdownText>>,
    /// The one note slot, when supplied.
    pub note: Option<TagValue<MarkdownText>>,
    /// The one format slot, when supplied.
    pub format: Option<TagValue<MarkdownText>>,
    /// The first data-object contribution that omitted `@format`.
    pub(crate) missing_data_format_span: Option<Span>,
    /// The one source slot, when supplied.
    pub source: Option<TagValue<MarkdownText>>,
    /// The one author slot, when supplied.
    pub author: Option<TagValue<MarkdownText>>,
    /// Inheritance requests awaiting a separate resolution layer.
    pub inheritance: Vec<InheritanceRequest>,
    /// Structured DESCRIPTION-derived author content for package topics.
    pub package_author: Option<PackageAuthor>,
    /// Structured DESCRIPTION-derived see-also content for package topics.
    pub package_see_also: Option<PackageSeeAlso>,
    /// DESCRIPTION diagnostics waiting for resolved fields at the Rd stage.
    pub package_metadata_diagnostics: Option<PackageMetadataDiagnosticState>,
}

/// DESCRIPTION-derived diagnostic context for one package topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageMetadataDiagnosticState {
    /// The first `"_PACKAGE"` sentinel span contributing to the topic.
    pub anchor: Span,
    /// Whether DESCRIPTION has no description and no fallback was suppressed.
    pub missing_description: bool,
    /// A malformed Authors@R value awaiting a final author-field check.
    pub authors_parse_error: Option<AuthorsParseError>,
}

/// Structured package author sections derived from `Authors@R`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageAuthor {
    pub maintainers: Vec<PackagePerson>,
    pub authors: Vec<PackagePerson>,
    pub other_contributors: Vec<PackagePerson>,
}

/// One author entry and its structured inline identity markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePerson {
    pub name: String,
    pub email: Option<String>,
    pub identities: Vec<PackageIdentity>,
    pub comments: Vec<PackageComment>,
    pub roles: Vec<String>,
}

/// A non-identity Authors@R comment retained for package author rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageComment {
    pub label: Option<String>,
    pub value: String,
}

/// An ORCID or ROR identity link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageIdentity {
    pub label: String,
    pub href: String,
}

/// Structured package links derived from URL and BugReports fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSeeAlso {
    pub urls: Vec<PackageLink>,
    pub bug_reports: Option<String>,
}

/// A package URL or DOI link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLink {
    pub target: String,
    pub doi: bool,
}

/// The kind of Rd topic being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdTopicKind {
    /// An ordinary object topic.
    #[default]
    Ordinary,
    /// A data object topic, which emits `\\docType{data}`.
    Data,
    /// A package-level topic, which emits `\\docType{package}`.
    Package,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TopicKindOrigin {
    pub kind: RdTopicKind,
    pub span: Span,
}

impl RdTopic {
    pub(in crate::model) fn new(name: DocName) -> Self {
        Self {
            name,
            kind: RdTopicKind::Ordinary,
            kind_origin: None,
            kind_conflict_reported: false,
            blocks: Vec::new(),
            aliases: Vec::new(),
            keywords: Vec::new(),
            title: None,
            description: None,
            description_suppressed: false,
            details: None,
            return_value: None,
            examples: None,
            usages: Vec::new(),
            formals: Vec::new(),
            params: Vec::new(),
            sections: Vec::new(),
            see_also: None,
            references: None,
            note: None,
            format: None,
            missing_data_format_span: None,
            source: None,
            author: None,
            inheritance: Vec::new(),
            package_author: None,
            package_see_also: None,
            package_metadata_diagnostics: None,
        }
    }

    /// Returns the ordered formal names available as this topic's inheritance
    /// target domain.
    ///
    /// Names are concatenated in contribution order and de-duplicated by
    /// first occurrence, matching roxygen2's `unique()` over concatenated
    /// formals. A non-function contribution is ignored when a function
    /// contribution is present. If any function contribution is structurally
    /// unreadable or contains an undecodable name, the corresponding failure
    /// state is returned instead of guessing from the remaining contributions.
    #[must_use]
    pub fn inheritance_formal_names(&self) -> FormalNames {
        let mut names = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut saw_function = false;

        for contribution in &self.formals {
            match &contribution.names {
                FormalNames::NotFunction => {}
                FormalNames::Known(formals) => {
                    saw_function = true;
                    for formal in formals {
                        if seen.insert(formal.name.0.clone()) {
                            names.push(formal.clone());
                        }
                    }
                }
                FormalNames::Unknown { span } => {
                    return FormalNames::Unknown { span: *span };
                }
                FormalNames::Undecodable { span } => {
                    return FormalNames::Undecodable { span: *span };
                }
            }
        }

        if saw_function {
            FormalNames::Known(names)
        } else {
            FormalNames::NotFunction
        }
    }
}

/// One formal name of a documented function, with the span that introduced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormalName {
    /// The decoded formal name.
    pub name: ParamName,
    /// The source span of the formal name.
    pub span: Span,
}

/// The formal-name facts contributed by one documented block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormalContribution {
    /// The block that supplied these facts.
    pub block: BlockRef,
    /// Whether and how the block's formal list was read.
    pub names: FormalNames,
}

/// How much is known about one block's formal names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormalNames {
    /// The documented object is not a function.
    NotFunction,
    /// The object is a function and its complete formal list was read.
    /// An empty vector therefore means a known function with no formals.
    Known(Vec<FormalName>),
    /// The object is a function, but its formal list has invalid structure.
    Unknown {
        /// A source span identifying the failed function structure.
        span: Span,
    },
    /// The formal list was structurally readable, but a name could not be
    /// decoded without guessing at R's escape semantics.
    Undecodable {
        /// The source span of the undecodable name.
        span: Span,
    },
}

/// One alias claim retained with the source span needed for package-wide
/// ownership diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// The alias text.
    pub name: DocName,
    /// The source span that claims this alias.
    pub span: Span,
}

/// A parameter description retained with the tag provenance needed by later
/// diagnostics and Rd origin mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDescription {
    /// The documented parameter name.
    pub name: ParamName,
    /// The normalized Markdown description.
    pub description: MarkdownText,
    /// The source origin of the `@param` tag.
    pub origin: TagOrigin,
}

/// A named section retained without constructing Rd AST nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSection {
    /// The source-level section title.
    pub title: MarkdownText,
    /// The section body awaiting Markdown conversion.
    pub body: MarkdownText,
    /// The source origin of the `@section` tag.
    pub origin: TagOrigin,
}

/// An S3 method declaration retained as typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDeclaration {
    /// The generic name.
    pub generic: Spanned<String>,
    /// The class name.
    pub class: Spanned<String>,
    /// The source origin of the `@method` tag.
    pub origin: TagOrigin,
}

/// One usage contribution from one documented block.
///
/// Keeping the method beside its usage prevents separate merge-order vectors
/// from losing the association needed to render an S3 method usage later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageContribution {
    /// The block that contributed this usage.
    pub block: BlockRef,
    /// The source span of the contributing documentation block.
    pub(crate) block_span: Span,
    /// The statically associated object name, when this contribution came
    /// from a named top-level binding.
    pub(crate) object: Option<RName>,
    /// The block's optional S3 method declaration.
    pub method: Option<MethodDeclaration>,
    /// The resolved usage, including an explicit absence.
    pub usage: ResolvedUsage,
}

/// Usage resolved from one documented block.
///
/// Explicit and generated values stay separate because explicit source is
/// user-authored while generated source is derived from a static signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedUsage {
    /// This block does not contribute usage.
    Absent,
    /// `@usage NULL` suppresses usage for this contribution.
    Suppressed(TagOrigin),
    /// Usage generated from a function object's formals.
    Generated(GeneratedUsage),
    /// Explicit source supplied by `@usage`.
    Explicit(TagValue<RCodeText>),
}

/// A deferred inheritance request from a topic.
///
/// Resolution is intentionally not performed here because it requires the
/// package-wide topic graph and, for external references, a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritanceRequest {
    /// Copy selected fields from another topic.
    Inherit {
        /// The requested source topic.
        target: InheritTarget,
        /// The fields requested by the tag.
        fields: InheritFields,
        /// The source origin of the request.
        origin: TagOrigin,
    },
    /// Copy all or selected parameters from another topic.
    InheritParams {
        /// The requested source topic.
        target: InheritTarget,
        /// Optional deferred argument selection.
        selection: Option<crate::tags::ArgSelection>,
        /// The source origin of the request.
        origin: TagOrigin,
    },
    /// Copy one named section from another topic.
    InheritSection {
        /// The requested source topic.
        target: InheritTarget,
        /// The requested section title and its source span.
        title: Spanned<MarkdownText>,
        /// The source origin of the request.
        origin: TagOrigin,
    },
}

/// A NAMESPACE directive awaiting validation, deduplication, and rendering.
///
/// The request is kept verbatim because those operations belong to a later
/// layer and different directives have different merge semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceRequest {
    /// The block that requested the directive.
    pub block: BlockRef,
    /// The typed directive and its original value.
    pub tag: NamespaceTag,
    /// The block's implicit object name, if static syntax supplied one.
    pub object: Option<RName>,
    /// Whether the block's implicit object is statically a function assignment.
    pub object_is_function: bool,
    /// The complete spelling span of the implicit binding name.
    pub object_spelling: Option<Span>,
    /// The block's first `@method` declaration, if present.
    pub method: Option<MethodDeclaration>,
}

#[cfg(test)]
mod tests {
    use super::{FormalNames, TopicKey};
    use crate::model::test_support::model;

    #[test]
    fn records_ordered_formals_for_an_ordinary_function() {
        let output = model(
            r#"#' title
f <- function(x, y) x
"#,
        );
        let topic = &output.package.topics[&TopicKey("f".into())];
        let FormalNames::Known(names) = topic.inheritance_formal_names() else {
            panic!("expected a known function formal list");
        };
        assert_eq!(
            names
                .iter()
                .map(|formal| formal.name.0.as_str())
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert_eq!(topic.formals.len(), 1);
    }

    #[test]
    fn records_ellipsis_as_an_ordinary_formal() {
        let output = model(
            r#"#' title
f <- function(x, ..., y) x
"#,
        );
        let topic = &output.package.topics[&TopicKey("f".into())];
        let FormalNames::Known(names) = topic.inheritance_formal_names() else {
            panic!("expected a known function formal list");
        };
        assert_eq!(
            names
                .iter()
                .map(|formal| formal.name.0.as_str())
                .collect::<Vec<_>>(),
            ["x", "...", "y"]
        );
    }

    #[test]
    fn distinguishes_a_known_function_with_no_formals_from_a_non_function() {
        let output = model(
            r#"#' title
f <- function() 1
"#,
        );
        let topic = &output.package.topics[&TopicKey("f".into())];
        assert_eq!(
            topic.inheritance_formal_names(),
            FormalNames::Known(Vec::new())
        );
    }

    #[test]
    fn marks_a_non_function_topic_as_not_a_function() {
        let output = model(
            r#"#' title
x <- 1
"#,
        );
        let topic = &output.package.topics[&TopicKey("x".into())];
        assert_eq!(topic.inheritance_formal_names(), FormalNames::NotFunction);
    }

    #[test]
    fn marks_an_undecodable_formal_without_emitting_a_second_usage_diagnostic() {
        let output = model("#' title\nf <- function(`x\\y`) x\n");
        let topic = &output.package.topics[&TopicKey("f".into())];
        assert!(matches!(
            topic.inheritance_formal_names(),
            FormalNames::Undecodable { .. }
        ));
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code == crate::diagnostic::DiagnosticCode::UsageGenerationFailed
                })
                .count(),
            1
        );
    }

    #[test]
    fn merged_formal_contributions_keep_first_occurrences_in_order() {
        let output = model(
            r#"#' @rdname shared
a <- function(x, y) x
#' @rdname shared
b <- function(y, z) y
"#,
        );
        let topic = &output.package.topics[&TopicKey("shared".into())];
        let FormalNames::Known(names) = topic.inheritance_formal_names() else {
            panic!("expected a known merged formal list");
        };
        assert_eq!(
            names
                .iter()
                .map(|formal| formal.name.0.as_str())
                .collect::<Vec<_>>(),
            ["x", "y", "z"]
        );
    }
}
