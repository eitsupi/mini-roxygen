//! Builds writer-valid Rd documents from the merged package model.

mod arguments;
mod examples_raw_rd;
mod origins;
mod output;
mod prose;
mod sections;
mod serialize;
mod topic;
mod usage;

pub use output::{GeneratedRd, RdBuildOutput};
#[cfg(test)]
pub(crate) use usage::rcode_nodes;

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::inherit::ResolvedPackageModel;
use crate::inline_r::InlineRSession;
#[cfg(test)]
use crate::inline_r::InlineRUsage;
use crate::markdown_conversion::{HelpLinkResolver, LinkResolution, MarkdownContext};
use crate::model::TopicKey;
use crate::source::{SourceMap, Span};

/// Resolution result supplied by callers that have package help metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum RdLinkResolution {
    /// The target is a topic in the current package.
    Local,
    /// No resolver was able to search for the target.
    ///
    /// This is distinct from [`Self::Unresolved`], which means a search was
    /// performed and found no target. An unchecked target is not an error and
    /// must not produce a diagnostic.
    Unchecked,
    /// The target is supplied by another package.
    External { package: String },
    /// No target could be found.
    Unresolved,
    /// More than one external package supplies the target.
    Ambiguous { packages: Vec<String> },
}

/// Resolves unqualified Markdown help links for Rd generation.
pub(crate) trait RdLinkResolver {
    fn resolve_unqualified(&self, topic: &str) -> RdLinkResolution;
}

#[cfg(test)]
struct EmptyLinks;

#[cfg(test)]
impl RdLinkResolver for EmptyLinks {
    fn resolve_unqualified(&self, _topic: &str) -> RdLinkResolution {
        RdLinkResolution::Unchecked
    }
}

pub(crate) struct LinkAdapter<'a> {
    pub(crate) links: &'a dyn RdLinkResolver,
}

impl HelpLinkResolver for LinkAdapter<'_> {
    fn resolve_unqualified(&self, topic: &str) -> LinkResolution {
        match self.links.resolve_unqualified(topic) {
            RdLinkResolution::Local => LinkResolution::Local,
            RdLinkResolution::Unchecked => LinkResolution::Unchecked,
            RdLinkResolution::External { package } => LinkResolution::External { package },
            RdLinkResolution::Unresolved => LinkResolution::Unresolved,
            RdLinkResolution::Ambiguous { packages } => LinkResolution::Ambiguous { packages },
        }
    }
}

/// Builds Rd output with no package-level link resolver.
#[must_use]
#[cfg(test)]
pub fn build_rd(package: &ResolvedPackageModel, sources: &SourceMap) -> RdBuildOutput {
    let links = EmptyLinks;
    let substitutions = crate::inline_r::InlineRSubstitutions::builtins()
        .expect("built-in substitutions should be valid");
    let usage = InlineRUsage::new();
    let session = InlineRSession::new(&substitutions, &usage);
    build_rd_with_context(package, sources, None, &links, &session)
}

/// Builds Rd output with Markdown package and help-link context.
///
/// Crate-private on purpose: it takes a session it does not own, and the
/// caller has to hand the same session to inheritance resolution and read the
/// unused-substitution report afterwards. That is a sequencing contract a
/// caller could get wrong, so the library does not offer it.
/// `pipeline::document_package_with_options` is the public orchestration
/// boundary, and it owns the contract whole.
#[must_use]
pub(crate) fn build_rd_with_context(
    package: &ResolvedPackageModel,
    sources: &SourceMap,
    current_package: Option<&str>,
    links: &dyn RdLinkResolver,
    session: &InlineRSession<'_>,
) -> RdBuildOutput {
    let adapter = LinkAdapter { links };
    let context = MarkdownContext {
        current_package,
        links: &adapter,
        inline_r_session: Some(session),
    };
    build_with_context(package, sources, &context)
}

pub(crate) fn deduplicate_inline_r_diagnostics(diagnostics: &mut Diagnostics) {
    let mut seen = BTreeSet::new();
    let mut retained = Diagnostics::new();
    for diagnostic in diagnostics.iter().cloned() {
        if matches!(
            diagnostic.code,
            DiagnosticCode::UnsupportedInlineR | DiagnosticCode::UndefinedInlineRSubstitution
        ) && !seen.insert((diagnostic.code, diagnostic.primary.span))
        {
            continue;
        }
        retained.push(diagnostic);
    }
    *diagnostics = retained;
}

fn build_with_context(
    package: &ResolvedPackageModel,
    sources: &SourceMap,
    context: &MarkdownContext<'_>,
) -> RdBuildOutput {
    let mut diagnostics = Diagnostics::new();
    // Every topic is built before any collision is judged. A topic that fails
    // on its own is not a claim on a filename, so deciding first would let it
    // take a valid topic down with it and swallow its own diagnostic too.
    let mut files = BTreeMap::new();
    for (key, topic) in &package.topics {
        if let Some((path, document, content)) =
            topic::build(key, topic, sources, context, &mut diagnostics)
        {
            files.insert(
                key.clone(),
                GeneratedRd {
                    relative_path: path,
                    document,
                    content,
                },
            );
        }
    }

    let mut claims = BTreeMap::<std::path::PathBuf, (TopicKey, Span)>::new();
    let mut colliding = BTreeSet::new();
    for (key, generated) in &files {
        let topic = &package.topics[key];
        let span = topic::anchor(topic).expect("a built topic has a source anchor");
        match claims.get(&generated.relative_path) {
            Some((previous, previous_span)) => {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagnosticCode::ConflictingRdFileName,
                        format!(
                            "topics `{}` and `{}` normalize to the same Rd file",
                            previous.as_str(),
                            key.as_str()
                        ),
                        Label::new(span, "normalized Rd filename is already used"),
                    )
                    .with_secondary(Label::new(
                        *previous_span,
                        "first topic using this filename",
                    )),
                );
                colliding.insert(previous.clone());
                colliding.insert(key.clone());
            }
            None => {
                claims.insert(generated.relative_path.clone(), (key.clone(), span));
            }
        }
    }
    files.retain(|key, _| !colliding.contains(key));

    RdBuildOutput { files, diagnostics }
}

#[cfg(test)]
mod tests;
