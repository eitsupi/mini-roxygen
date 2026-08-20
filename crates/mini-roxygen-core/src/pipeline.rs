//! The composed source-to-output pipeline for one in-memory package.

use std::collections::BTreeSet;

use crate::arity_adapter;
use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::inherit::{
    DocumentationError, DocumentationProvider, InheritableTopic, InheritanceOptions,
    LocalDocumentationProvider, TopicExistence, TopicRequest,
};
use crate::inline_r::{InlineRSession, InlineRSubstitutions, InlineRUsage};
use crate::model::{
    BlockRef, DocumentedBlock, build_package_model_with_metadata_bindings_and_registrations,
};
use crate::namespace::{self, EmptyS3GenericProvider, NamespaceBuildOutput, S3GenericProvider};
use crate::package::PackageInputs;
use crate::r_parse;
use crate::rd::{self, RdBuildOutput};
use crate::s3_register::{S3RegistrarSet, S3RegistrationFact};
use crate::source::FileId;
use crate::tags::{TagParseOptions, UnknownTagPolicy, parse_block};

/// All generated package outputs and diagnostics from the composed pipeline.
#[derive(Debug)]
pub struct PackageOutput {
    /// Generated Rd files.
    pub rd: RdBuildOutput,
    /// Generated NAMESPACE contents.
    pub namespace: NamespaceBuildOutput,
    analysis_diagnostics: Diagnostics,
}

/// Configuration supplied to the documentation pipeline by its caller.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentOptions {
    /// The already validated static substitutions for inline R expressions.
    pub inline_r_substitutions: InlineRSubstitutions,
    /// The validated S3 registrar signatures used for static registration facts.
    pub s3_registrars: S3RegistrarSet,
}

impl PackageOutput {
    /// Iterates over diagnostics in pipeline order without copying them.
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.analysis_diagnostics
            .iter()
            .chain(self.rd.diagnostics.iter())
            .chain(self.namespace.diagnostics.iter())
    }

    /// Returns whether any pipeline stage reported an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.analysis_diagnostics.has_errors()
            || self.rd.diagnostics.has_errors()
            || self.namespace.diagnostics.has_errors()
    }
}

/// Documents all source files in a package represented by an in-memory map.
///
/// Every analysis and generation stage returns partial output plus diagnostics,
/// so the whole chain runs even after an earlier stage reports an error.
#[must_use]
pub fn document_package(inputs: &PackageInputs) -> PackageOutput {
    let options = DocumentOptions {
        inline_r_substitutions: InlineRSubstitutions::builtins()
            .expect("built-in substitutions should be valid"),
        s3_registrars: S3RegistrarSet::default(),
    };
    document_package_with_options(inputs, &options)
}

/// Documents a package using caller-supplied, validated options.
///
/// This is the option-aware entry point into the library. One call owns one
/// inline R session, and that session spans inheritance resolution, the Rd
/// build, and the report of substitutions the package never used. The stages
/// it drives are deliberately not callable on their own: a stage given its own
/// session would account for usage separately and report entries as unused
/// that another stage had matched.
#[must_use]
pub fn document_package_with_options(
    inputs: &PackageInputs,
    options: &DocumentOptions,
) -> PackageOutput {
    document_package_with_options_and_s3_provider(inputs, options, &EmptyS3GenericProvider)
}

/// Documents a package using caller-supplied S3 generic facts.
#[must_use]
pub fn document_package_with_options_and_s3_provider(
    inputs: &PackageInputs,
    options: &DocumentOptions,
    s3_provider: &dyn S3GenericProvider,
) -> PackageOutput {
    document_package_with_options_and_providers(
        inputs,
        options,
        s3_provider,
        &EmptyDocumentationProvider,
        &InheritanceOptions::default(),
    )
}

/// Documents a package with independent S3 metadata and documentation
/// providers plus an explicit external-inheritance policy.
#[must_use]
pub fn document_package_with_options_and_providers(
    inputs: &PackageInputs,
    options: &DocumentOptions,
    s3_provider: &dyn S3GenericProvider,
    documentation_provider: &dyn DocumentationProvider,
    inheritance_options: &InheritanceOptions,
) -> PackageOutput {
    let usage = InlineRUsage::new();
    let session = InlineRSession::new(&options.inline_r_substitutions, &usage);
    let sources = &inputs.sources;
    let mut analysis_diagnostics = Diagnostics::new();
    let mut blocks = Vec::new();
    let mut bindings = Vec::new();
    let mut registrations = Vec::<S3RegistrationFact>::new();

    // SourceMap registration order is deterministic. Walking FileIds by index
    // keeps this pipeline independent of any map implementation details.
    for index in 0..sources.len() {
        let file = FileId::new(u32::try_from(index).expect("source map has too many files"));
        let source = sources.get(file).expect("registered source file");
        let parsed = arity_adapter::parse(source, file);
        analysis_diagnostics.extend(parsed.diagnostics.iter().cloned());
        let (facts, registrar_diagnostics) =
            crate::s3_register::extract(&parsed.calls, &options.s3_registrars);
        registrations.extend(facts);
        analysis_diagnostics.extend(registrar_diagnostics.iter().cloned());
        let object_index = r_parse::build_object_index(parsed, file);
        bindings.extend(object_index.bindings.iter().cloned());

        // build_object_index consumes its ParsedFile, so retain the fixture's
        // second parse for looking up the original documentation blocks.
        let parsed = arity_adapter::parse(source, file);
        for object in object_index.documented {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("indexed object has its raw documentation block");
            let (tags, tag_diagnostics) = parse_block(
                source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Warn),
            );
            analysis_diagnostics.extend(tag_diagnostics.iter().cloned());
            blocks.push(DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            });
        }
    }

    let mut model = build_package_model_with_metadata_bindings_and_registrations(
        sources,
        blocks,
        &inputs.metadata,
        bindings,
        registrations,
    );
    crate::namespace::classify_usage_methods(&mut model.package, sources, s3_provider);
    analysis_diagnostics.extend(model.diagnostics.iter().cloned());
    let local_links = PackageLocalLinks::new(&model.package);
    let local_provider = LocalDocumentationProvider::new(&model.package);
    let inheritance = crate::inherit::resolve_inheritance_with_substitutions(
        &model.package,
        Some(inputs.metadata.package()),
        &local_links,
        &ComposedDocumentationProvider {
            local: &local_provider,
            external: documentation_provider,
        },
        inheritance_options,
        &session,
    );
    analysis_diagnostics.extend(inheritance.diagnostics.iter().cloned());

    let mut rd = rd::build_rd_with_context(
        &inheritance.package,
        sources,
        Some(inputs.metadata.package()),
        &local_links,
        &session,
    );
    rd::deduplicate_inline_r_diagnostics(&mut rd.diagnostics);
    analysis_diagnostics.extend(
        options
            .inline_r_substitutions
            .unused_diagnostics(&usage)
            .iter()
            .cloned(),
    );
    let namespace = namespace::build_namespace_with_sources_and_provider(
        &model.package,
        sources,
        Some(inputs.metadata.package()),
        s3_provider,
    );

    PackageOutput {
        rd,
        namespace,
        analysis_diagnostics,
    }
}

struct EmptyDocumentationProvider;

impl DocumentationProvider for EmptyDocumentationProvider {
    fn get_topic(
        &self,
        _request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        Ok(None)
    }
}

struct ComposedDocumentationProvider<'a> {
    local: &'a LocalDocumentationProvider<'a>,
    external: &'a dyn DocumentationProvider,
}

impl DocumentationProvider for ComposedDocumentationProvider<'_> {
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        match request {
            TopicRequest::Local { .. } => self.local.get_topic(request),
            TopicRequest::External { .. } => self.external.get_topic(request),
        }
    }

    fn topic_exists(&self, package: &str, alias: &str) -> TopicExistence {
        self.external.topic_exists(package, alias)
    }
}

struct PackageLocalLinks {
    aliases: BTreeSet<String>,
}

impl PackageLocalLinks {
    fn new(package: &crate::model::PackageModel) -> Self {
        let aliases = package
            .topics
            .values()
            .flat_map(|topic| topic.aliases.iter())
            .map(|alias| alias.name.0.clone())
            .collect();
        Self { aliases }
    }
}

impl rd::RdLinkResolver for PackageLocalLinks {
    fn resolve_unqualified(&self, topic: &str) -> rd::RdLinkResolution {
        if self.aliases.contains(topic) {
            rd::RdLinkResolution::Local
        } else {
            rd::RdLinkResolution::Unchecked
        }
    }
}

trait DiagnosticsExt {
    fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>);
}

impl DiagnosticsExt for Diagnostics {
    fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        for diagnostic in diagnostics {
            self.push(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{
        ComposedDocumentationProvider, DocumentOptions, document_package,
        document_package_with_options, document_package_with_options_and_providers,
    };
    use crate::diagnostic::DiagnosticCode;
    use crate::inherit::{
        DocumentationError, DocumentationErrorKind, DocumentationIdentity, DocumentationOrigin,
        DocumentationProvider, ExternalInheritancePolicy, ExternalPolicySource, InheritableContent,
        InheritableFields, InheritableParamGroup, InheritableParamLabel, InheritableSection,
        InheritableTopic, InheritanceOptions, InheritanceTrace, LocalDocumentationProvider,
        ResolvedContent, TopicExistence, TopicRequest,
    };
    use crate::model::{PackageModel, TopicKey};
    use crate::namespace::EmptyS3GenericProvider;
    use crate::package::{PackageInputs, PackageMetadata};
    use crate::rd::RdLinkResolution;
    use crate::s3_register::{S3RegistrarRole, S3RegistrarSet, S3RegistrarSignature};
    use crate::source::{FileId, SourceFile, SourceMap};
    use crate::tags::InheritField;

    fn inputs(sources: SourceMap) -> PackageInputs {
        PackageInputs {
            sources,
            metadata: PackageMetadata::new("currentPackage", None)
                .expect("test package name should be valid"),
        }
    }

    struct PipelineDocumentationProvider;

    fn external_content(component: InheritField, text: &str) -> ResolvedContent {
        ResolvedContent {
            value: InheritableContent::Rd(vec![rd_ast::RdNode::Text(text.to_owned())]),
            provenance: InheritanceTrace {
                source: DocumentationOrigin::External {
                    package: "pkg".into(),
                    topic: "donor".into(),
                    component,
                },
                requests: Vec::new(),
            },
        }
    }

    impl DocumentationProvider for PipelineDocumentationProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { topic, .. } = request else {
                return Ok(None);
            };
            if topic.0 == "missing" {
                return Err(DocumentationError {
                    kind: DocumentationErrorKind::TopicUnreadable,
                    package: Some("pkg".into()),
                    topic: Some(topic.0.clone()),
                    detail: "test provider failure".into(),
                });
            }
            if topic.0 != "donor" {
                return Ok(None);
            }
            Ok(Some(InheritableTopic {
                identity: DocumentationIdentity::External {
                    package: "pkg".into(),
                    topic: "donor".into(),
                },
                params: vec![
                    InheritableParamGroup {
                        names: vec![crate::tags::ParamName("x".into())],
                        label: InheritableParamLabel::Generated,
                        description: external_content(InheritField::Params, "donor x"),
                    },
                    InheritableParamGroup {
                        names: vec![crate::tags::ParamName("y".into())],
                        label: InheritableParamLabel::Generated,
                        description: external_content(InheritField::Params, "donor y"),
                    },
                ],
                fields: InheritableFields {
                    title: Some(external_content(InheritField::Title, "donor title")),
                    description: Some(external_content(
                        InheritField::Description,
                        "donor description",
                    )),
                    details: Some(external_content(InheritField::Details, "donor details")),
                    return_value: Some(external_content(InheritField::Return, "donor return")),
                    see_also: Some(external_content(InheritField::SeeAlso, "donor seealso")),
                    references: Some(external_content(
                        InheritField::References,
                        "donor references",
                    )),
                    examples: Some(ResolvedContent {
                        value: InheritableContent::Rd(vec![rd_ast::RdNode::RCode(
                            "donor examples".into(),
                        )]),
                        provenance: InheritanceTrace {
                            source: DocumentationOrigin::External {
                                package: "pkg".into(),
                                topic: "donor".into(),
                                component: InheritField::Examples,
                            },
                            requests: Vec::new(),
                        },
                    }),
                    author: Some(external_content(InheritField::Author, "donor author")),
                    source: Some(external_content(InheritField::Source, "donor source")),
                    note: Some(external_content(InheritField::Note, "donor note")),
                    format: Some(external_content(InheritField::Format, "donor format")),
                },
                sections: vec![InheritableSection {
                    title: external_content(InheritField::Sections, "donor section"),
                    body: external_content(InheritField::Sections, "donor section body"),
                }],
                requests: Vec::new(),
            }))
        }
    }

    #[test]
    fn param_continuation_indent_is_removed_before_rd_serialization() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/continuation.R"),
            concat!(
                "#' @title Continuation\n",
                "#' @param strict Use `how =\n",
                "#'   \"horizontal_extend\"`\n",
                "f <- function(strict) NULL\n",
            )
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(
            !output.has_errors(),
            "unexpected diagnostics: {:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
        let target = output
            .rd
            .files
            .get(&TopicKey("f".into()))
            .expect("function topic should be generated");
        assert!(
            target
                .content
                .contains(r#"\code{how = "horizontal_extend"}"#),
            "serialized parameter description: {}",
            target.content
        );
    }

    #[test]
    fn level_one_headings_flatten_in_generated_description_and_details() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/headings.R"),
            r#"#' @title Heading topic
#' @description
#' # Description heading
#'
#' Description text.
#' @details
#' # Details heading
#'
#' Details text.
topic <- function() NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(!output.has_errors());
        let target = output
            .rd
            .files
            .get(&TopicKey("topic".into()))
            .expect("topic Rd should be generated");
        insta::assert_snapshot!(target.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/headings.R
\name{topic}
\alias{topic}
\title{Heading topic}
\usage{
topic()
}
\description{
Description heading

Description text.
}
\details{
Details heading

Details text.
}
"###);
        crate::rd_oracle::assert_r_accepts(&target.content);

        let heading_diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedMarkdownHeading)
            .collect::<Vec<_>>();
        assert_eq!(heading_diagnostics.len(), 2);
        assert!(
            heading_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == crate::diagnostic::Severity::Warning)
        );
    }

    #[test]
    fn external_inheritance_is_injected_with_local_first_and_partial_warning_behavior() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/inherit.R"),
            r#"#' @name target
#' @title Local target title
#' @description Local target description
#' @param x Local x
#' @inheritParams pkg::donor
#' @inherit pkg::donor
target <- function(x, y) NULL

#' @title Warned
#' @export
#' @inherit pkg::missing title
warned <- function() NULL
"#
            .to_owned(),
        ));
        let options = DocumentOptions {
            inline_r_substitutions: crate::inline_r::InlineRSubstitutions::builtins()
                .expect("built-in substitutions should be valid"),
            s3_registrars: S3RegistrarSet::default(),
        };
        let inheritance_options = InheritanceOptions {
            external: ExternalInheritancePolicy::BestEffort,
            external_source: ExternalPolicySource::Explicit,
        };
        let output = document_package_with_options_and_providers(
            &inputs(sources),
            &options,
            &EmptyS3GenericProvider,
            &PipelineDocumentationProvider,
            &inheritance_options,
        );

        let target = output
            .rd
            .files
            .get(&TopicKey("target".into()))
            .expect("target Rd should be generated");
        insta::assert_snapshot!(target.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/inherit.R
\name{target}
\alias{target}
\title{Local target title}
\format{
donor format
}
\source{
donor source
}
\usage{
target(x, y)
}
\arguments{
\item{x}{Local x}

\item{y}{donor y}
}
\value{
donor return
}
\description{
Local target description
}
\details{
donor details
}
\note{
donor note
}
\section{donor section}{
 donor section body
}

\examples{
donor examples
}
\references{
donor references
}
\seealso{
donor seealso
}
\author{
donor author
}
"###);
        let warned = output
            .rd
            .files
            .get(&TopicKey("warned".into()))
            .expect("warned Rd should be generated");
        insta::assert_snapshot!(warned.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/inherit.R
\name{warned}
\alias{warned}
\title{Warned}
\usage{
warned()
}
\description{
Warned
}
"###);
        let diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
            .map(|diagnostic| format!("{:?}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(diagnostics, @r###"
UnresolvedInherit: could not load external inheritance topic `pkg::missing`: test provider failure
"###);
        assert!(!output.has_errors());
        insta::assert_snapshot!(output.namespace.content, @r###"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand

export(warned)
"###);
    }

    #[test]
    fn direct_alias_formals_support_transitive_inherited_parameters() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/aliases.R"),
            r#"base_function <- function(first, second = "default") NULL

#' @title Alias function
#' @inheritParams donor_function
alias_function <- base_function

#' @title Donor function
#' @param first First value.
#' @param second Second value.
donor_function <- function(first, second = "default") NULL

#' @inherit alias_function title params
consumer <- function(first, second = "default") NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        let final_doc = output
            .rd
            .files
            .get(&TopicKey("consumer".into()))
            .expect("transitive alias target should be generated");
        insta::assert_snapshot!(final_doc.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/aliases.R
\name{consumer}
\alias{consumer}
\title{Alias function}
\usage{
consumer(first, second = "default")
}
\arguments{
\item{first}{First value.}

\item{second}{Second value.}
}
\description{
Alias function
}
"###);
        insta::assert_snapshot!(output.namespace.content, @r###"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
"###);
    }

    #[test]
    fn s3_registration_supplies_rd_method_metadata_without_namespace_directive() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::generic.with.dots", "class.with.dots")
#' @title Registered method
#' @exportS3Method NULL
generic.with.dots.class.with.dots <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        let content = output
            .rd
            .files
            .values()
            .next()
            .expect("registered method topic")
            .content
            .clone();
        insta::assert_snapshot!(content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/register.R
\name{generic.with.dots.class.with.dots}
\alias{generic.with.dots.class.with.dots}
\title{Registered method}
\usage{
\method{generic.with.dots}{class.with.dots}(x)
}
\description{
Registered method
}
"###);
        insta::assert_snapshot!(output.namespace.content, @r###"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
"###);
        assert!(
            !output.has_errors(),
            "{:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
    }

    #[test]
    fn dynamic_s3_registration_does_not_add_rd_method_directive() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#".onLoad <- function(libname, pkgname) {
  for (class in c("class_one", "class_two")) {
    s3_register("dependency::generic", class)
  }
  s3_register(paste0("dependency::", pkgname), "class")
}
#' @title Dynamic registration
generic.class <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        let content = output
            .rd
            .files
            .values()
            .next()
            .expect("documented function topic")
            .content
            .clone();
        insta::assert_snapshot!(content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/register.R
\name{generic.class}
\alias{generic.class}
\title{Dynamic registration}
\usage{
generic.class(x)
}
\description{
Dynamic registration
}
"###);
        insta::assert_snapshot!(output.namespace.content, @r###"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
"###);
        assert!(
            !output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::InvalidS3Registration),
            "dynamic registrations must not be diagnosed: {:?}",
            output.diagnostics().collect::<Vec<_>>()
        );
        let dynamic_diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DynamicS3Registration)
            .collect::<Vec<_>>();
        assert_eq!(dynamic_diagnostics.len(), 2);
        assert!(
            dynamic_diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == crate::diagnostic::Severity::Info)
        );
        assert!(!output.has_errors());
    }

    #[test]
    fn s3_registration_requires_a_local_function_target() {
        for source in [
            r#"s3_register("pkg::generic", "class")
"#,
            r#"method <- 1
s3_register("pkg::generic", "class", method)
"#,
        ] {
            let mut sources = SourceMap::new();
            sources.add_file(SourceFile::new(
                PathBuf::from("R/register.R"),
                source.to_owned(),
            ));
            let output = document_package(&inputs(sources));
            assert!(output.has_errors(), "expected invalid target: {source}");
            assert!(
                output
                    .diagnostics()
                    .any(|diagnostic| { diagnostic.code == DiagnosticCode::InvalidS3Registration })
            );
        }
    }

    #[test]
    fn documented_registered_target_warns_without_export_or_suppression() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::generic", "class")
#' @title Registered method
generic.class <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        let warning = output
            .diagnostics()
            .find(|diagnostic| diagnostic.code == DiagnosticCode::UnexportedS3Method)
            .expect("missing export warning");
        assert_eq!(warning.severity, crate::diagnostic::Severity::Warning);
        assert!(
            warning
                .help
                .as_deref()
                .is_some_and(|help| help.contains("NULL"))
        );
        assert!(!output.has_errors());
    }

    #[test]
    fn registration_metadata_does_not_drive_export_namespace_inference() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::generic", "class")
#' @title Registered method
#' @export
generic.class <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        assert!(!output.namespace.content.contains("S3method("));
        assert!(!output.has_errors());
    }

    #[test]
    fn undocumented_registered_target_is_silent() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::generic", "class")
generic.class <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        assert!(
            !output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::UnexportedS3Method)
        );
        assert!(!output.has_errors());
    }

    #[test]
    fn unmatched_null_export_is_an_actionable_error() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"#' @title Unmatched method
#' @exportS3Method NULL
some_method <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        let diagnostic = output
            .diagnostics()
            .find(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedS3MethodMetadata)
            .expect("missing unresolved method metadata error");
        assert_eq!(diagnostic.severity, crate::diagnostic::Severity::Error);
        assert!(
            diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("registrar"))
        );
        assert!(output.has_errors());
    }

    #[test]
    fn duplicate_registration_target_bindings_are_rejected() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"method <- function(x) x
method <- function(x) x
s3_register("pkg::generic", "class", method)
#' @title Registered method
#' @exportS3Method NULL
method <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        assert!(
            output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::InvalidS3Registration)
        );
        assert!(output.has_errors());
    }

    #[test]
    fn multiple_registration_pairs_require_explicit_method_metadata() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::first", "class", method)
s3_register("pkg::second", "class", method)
#' @title Ambiguous method
method <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        let diagnostic = output
            .diagnostics()
            .find(|diagnostic| diagnostic.code == DiagnosticCode::AmbiguousS3Registration)
            .expect("missing ambiguous registration diagnostic");
        assert_eq!(diagnostic.secondary.len(), 2);
        assert!(
            diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("@method"))
        );
        assert!(output.has_errors());
    }

    #[test]
    fn identical_registration_pairs_deduplicate_with_first_provenance() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"s3_register("pkg::generic", "class", method)
s3_register("pkg::generic", "class", method)
#' @title Registered method
#' @exportS3Method NULL
method <- function(x) x
"#
            .to_owned(),
        ));
        let output = document_package(&inputs(sources));
        assert!(
            !output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::AmbiguousS3Registration)
        );
        let content = output
            .rd
            .files
            .values()
            .next()
            .expect("registered method topic")
            .content
            .clone();
        assert!(content.contains(r"\method{generic}{class}(x)"));
        assert!(!output.has_errors());
    }

    #[test]
    fn custom_registrar_signature_reaches_rd_without_namespace_directive() {
        let signature = S3RegistrarSignature::new(
            "register_s3_method",
            vec![
                S3RegistrarRole::Class,
                S3RegistrarRole::Generic,
                S3RegistrarRole::Method,
            ],
        )
        .expect("valid registrar signature");
        let options = DocumentOptions {
            inline_r_substitutions: crate::inline_r::InlineRSubstitutions::builtins()
                .expect("built-in substitutions should be valid"),
            s3_registrars: S3RegistrarSet::with_additions([signature])
                .expect("custom signature should compose"),
        };
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"register_s3_method(method = method, class = "class", generic = "pkg::generic")
#' @title Registered method
#' @exportS3Method NULL
method <- function(x) x
"#
            .to_owned(),
        ));
        let output = super::document_package_with_options(&inputs(sources), &options);
        let content = output
            .rd
            .files
            .values()
            .next()
            .expect("registered method topic")
            .content
            .clone();
        insta::assert_snapshot!(content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/register.R
\name{method}
\alias{method}
\title{Registered method}
\usage{
\method{generic}{class}(x)
}
\description{
Registered method
}
"###);
        insta::assert_snapshot!(output.namespace.content, @r###"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
"###);
        assert!(!output.has_errors());
    }

    #[test]
    fn default_documentation_uses_builtins_and_options_can_override_a_badge() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/badge.R"),
            r#"#' @title Badge
#' Uses `r lifecycle::badge("experimental")`.
badge <- function() NULL
"#
            .to_owned(),
        ));

        let default_output = document_package(&inputs(sources.clone()));
        let default_content = default_output
            .rd
            .files
            .values()
            .next()
            .expect("badge topic")
            .content
            .clone();
        assert!(default_content.contains("lifecycle-experimental.svg"));

        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([(
                r#"lifecycle::badge("experimental")"#.to_owned(),
                r#"\strong{custom badge}"#.to_owned(),
            )]),
            Some("test options".to_owned()),
        )
        .expect("override should validate");
        let options = DocumentOptions {
            inline_r_substitutions: substitutions,
            s3_registrars: S3RegistrarSet::default(),
        };
        let overridden_output = document_package_with_options(&inputs(sources), &options);
        let overridden_content = overridden_output
            .rd
            .files
            .values()
            .next()
            .expect("badge topic")
            .content
            .clone();
        assert!(overridden_content.contains(r"\strong{custom badge}"));
        assert!(!overridden_content.contains("lifecycle-experimental.svg"));
    }

    #[test]
    fn package_output_exposes_each_builder_and_stage_in_order() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/pipeline.R"),
            r#"#' @title First
#' @unknown-tag warning
#' @param x first description
same <- function(x) NULL

#' @title First
#' @param x second description
same <- function(x) NULL

#' @title Child
#' @inheritParams absent
child <- function(x) NULL

#' @title Inline
#' Description with `r 1 + 1`.
inline <- function() NULL

#' @importFrom utils
NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(!output.rd.files.is_empty());
        assert!(
            output
                .namespace
                .content
                .starts_with("# Generated by mini-roxygen")
        );

        let codes = output
            .diagnostics()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        let positions = [
            DiagnosticCode::UnknownTag,
            DiagnosticCode::ConflictingParamDescription,
            DiagnosticCode::UnresolvedInherit,
            DiagnosticCode::UndefinedInlineRSubstitution,
            DiagnosticCode::InvalidNamespaceDirective,
        ]
        .map(|code| {
            codes
                .iter()
                .position(|actual| *actual == code)
                .unwrap_or_else(|| panic!("missing diagnostic {code:?}: {codes:?}"))
        });
        assert!(positions.windows(2).all(|window| window[0] < window[1]));
        assert!(output.has_errors());
    }

    #[test]
    fn package_metadata_is_forwarded_to_namespace_generation() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/import.R"),
            r#"#' @importFrom currentPackage name
NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(!output.namespace.content.contains("importFrom"));
        assert!(
            output
                .diagnostics()
                .any(|diagnostic| { diagnostic.code == DiagnosticCode::SelfImport })
        );
    }

    #[test]
    fn export_only_blocks_do_not_check_undocumented_formals() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/export.R"),
            r#"#' @export
f <- function(x, y) x
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(
            output
                .diagnostics()
                .all(|diagnostic| { diagnostic.code != DiagnosticCode::MissingParam })
        );
    }

    #[test]
    fn package_local_help_links_resolve_without_changing_rd_markup() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/links.R"),
            r#"#' Alpha topic
#'
#' See [beta()] and [beta] for details.
#' @export
alpha <- function() NULL

#' Beta topic
#'
#' Plain body.
#' @export
beta <- function() NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        let alpha = output
            .rd
            .files
            .values()
            .find(|file| file.relative_path.ends_with("alpha.Rd"))
            .expect("alpha.Rd");
        assert!(alpha.content.contains(r"\code{\link[=beta]{beta()}}"));
        assert!(alpha.content.contains(r"\link{beta}"));
        assert!(
            !output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::AmbiguousExternalAlias)
        );
    }

    #[test]
    fn reused_options_do_not_share_inline_r_usage() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            Some("test options".to_owned()),
        )
        .expect("configuration should validate");
        let options = DocumentOptions {
            inline_r_substitutions: substitutions,
            s3_registrars: S3RegistrarSet::default(),
        };

        let mut first = SourceMap::new();
        first.add_file(SourceFile::new(
            PathBuf::from("R/first.R"),
            "#' @title First\n#' Description with `r known()`.\nfirst <- function() NULL\n"
                .to_owned(),
        ));
        let first_output = document_package_with_options(&inputs(first), &options);
        assert_eq!(
            first_output
                .diagnostics()
                .filter(|diagnostic| {
                    diagnostic.code == DiagnosticCode::UnusedInlineRSubstitution
                })
                .count(),
            0
        );

        let mut second = SourceMap::new();
        second.add_file(SourceFile::new(
            PathBuf::from("R/second.R"),
            "#' @title Second\nsecond <- function() NULL\n".to_owned(),
        ));
        let second_output = document_package_with_options(&inputs(second), &options);
        let warnings = second_output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnusedInlineRSubstitution)
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("known()"));
    }

    #[test]
    fn unused_inline_r_usage_does_not_affect_a_later_match() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            Some("test options".to_owned()),
        )
        .expect("configuration should validate");
        let options = DocumentOptions {
            inline_r_substitutions: substitutions,
            s3_registrars: S3RegistrarSet::default(),
        };

        let mut first = SourceMap::new();
        first.add_file(SourceFile::new(
            PathBuf::from("R/first.R"),
            "#' @title First\nfirst <- function() NULL\n".to_owned(),
        ));
        let first_output = document_package_with_options(&inputs(first), &options);
        assert_eq!(
            first_output
                .diagnostics()
                .filter(|diagnostic| {
                    diagnostic.code == DiagnosticCode::UnusedInlineRSubstitution
                })
                .count(),
            1
        );

        let mut second = SourceMap::new();
        second.add_file(SourceFile::new(
            PathBuf::from("R/second.R"),
            "#' @title Second\n#' Description with `r known()`.\nsecond <- function() NULL\n"
                .to_owned(),
        ));
        let second_output = document_package_with_options(&inputs(second), &options);
        assert_eq!(
            second_output
                .diagnostics()
                .filter(|diagnostic| {
                    diagnostic.code == DiagnosticCode::UnusedInlineRSubstitution
                })
                .count(),
            0
        );
    }

    #[test]
    fn document_options_are_unchanged_by_use() {
        let substitutions = crate::inline_r::InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            Some("test options".to_owned()),
        )
        .expect("configuration should validate");
        let options = DocumentOptions {
            inline_r_substitutions: substitutions,
            s3_registrars: S3RegistrarSet::default(),
        };
        let clone = options.clone();
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/options.R"),
            "#' @title Options\n#' Description with `r known()`.\noptions <- function() NULL\n"
                .to_owned(),
        ));

        let _ = document_package_with_options(&inputs(sources), &options);
        assert_eq!(options, clone);
    }

    #[test]
    fn package_local_links_use_explicit_aliases_and_leave_suppressed_keys_unchecked() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/links.R"),
            r#"#' Probe topic
#'
#' See [friendly], [hidden], and [mean()] for details.
probe <- function() NULL

#' Friendly topic
#' @aliases friendly
friendly_topic <- function() NULL

#' Hidden topic
#' @aliases NULL
hidden <- function() NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        assert!(
            !output
                .diagnostics()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::AmbiguousExternalAlias)
        );

        let model = crate::model::test_support::model(
            r#"#' Friendly topic
#' @aliases friendly
friendly_topic <- function() NULL

#' Hidden topic
#' @aliases NULL
hidden <- function() NULL
"#,
        );
        let links = super::PackageLocalLinks::new(&model.package);
        assert_eq!(
            crate::rd::RdLinkResolver::resolve_unqualified(&links, "friendly"),
            RdLinkResolution::Local
        );
        assert_eq!(
            crate::rd::RdLinkResolver::resolve_unqualified(&links, "hidden"),
            RdLinkResolution::Unchecked
        );
        assert_eq!(
            crate::rd::RdLinkResolver::resolve_unqualified(&links, "missing"),
            RdLinkResolution::Unchecked
        );
    }

    #[test]
    fn inherited_multiline_inline_r_is_recovered_once_per_source_occurrence() {
        let mut sources = SourceMap::new();
        let source = r#"#' @title Donor
#' @param x Uses `r "a
#' b"`.
donor <- function(x) NULL

#' @title Child One
#' @inheritParams donor
child_one <- function(x) NULL

#' @title Child Two
#' @inheritParams donor
child_two <- function(x) NULL
"#
        .to_owned();
        let donor_code_start = source.find("`r \"a").expect("donor inline R span") as u32;
        sources.add_file(SourceFile::new(
            PathBuf::from("R/inherit-multiline-inline-r.R"),
            source,
        ));

        let output = document_package(&inputs(sources.clone()));
        let diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedInlineR)
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "multi-line inline R code is not supported"
        );
        assert_eq!(diagnostics[0].primary.span.file, FileId::new(0));
        assert_eq!(diagnostics[0].primary.span.range.start(), donor_code_start);
        assert_eq!(
            sources.span_text(diagnostics[0].primary.span),
            Some("`r \"a\n")
        );
        assert_eq!(
            output
                .diagnostics()
                .filter(|diagnostic| {
                    diagnostic.code == DiagnosticCode::UndefinedInlineRSubstitution
                })
                .count(),
            0
        );
    }

    #[test]
    fn distinct_multiline_inline_r_occurrences_are_not_deduplicated() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/distinct-multiline-inline-r.R"),
            r#"#' @title Two expressions
#' @param x First `r "a
#' b"`.
#' @param y Second `r "a
#' b"`.
two_expressions <- function(x, y) NULL
"#
            .to_owned(),
        ));

        let output = document_package(&inputs(sources));
        let diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedInlineR)
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2);
        assert_ne!(diagnostics[0].primary.span, diagnostics[1].primary.span);
    }

    #[test]
    fn inherited_inline_r_diagnostics_are_deduplicated_after_topic_gating() {
        let mut sources = SourceMap::new();
        let source = format!(
            r#"#' @title Donor
            #' @param x Uses {}r missing(){}.
            donor <- function(x) NULL

            #' @title Child
            #' @inheritParams donor
            child <- function(x) NULL
"#,
            char::from(96),
            char::from(96)
        );
        sources.add_file(SourceFile::new(
            PathBuf::from("R/inherit-inline-r.R"),
            source,
        ));

        let output = document_package(&inputs(sources));
        let diagnostics = output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UndefinedInlineRSubstitution)
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
    }

    struct FixedExistenceProvider(TopicExistence);

    impl DocumentationProvider for FixedExistenceProvider {
        fn get_topic(
            &self,
            _request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            Ok(None)
        }

        fn topic_exists(&self, _package: &str, _alias: &str) -> TopicExistence {
            self.0
        }
    }

    #[test]
    fn composed_provider_delegates_topic_exists_to_external() {
        let package = PackageModel::default();
        let local = LocalDocumentationProvider::new(&package);

        for existence in [
            TopicExistence::Known(true),
            TopicExistence::Known(false),
            TopicExistence::Unavailable,
        ] {
            let external = FixedExistenceProvider(existence);
            let composed = ComposedDocumentationProvider {
                local: &local,
                external: &external,
            };
            assert_eq!(composed.topic_exists("pkg", "alias"), existence);
        }
    }
}
