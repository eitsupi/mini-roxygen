//! Conservative package-local S3 classification for implicit exports.

use std::collections::{BTreeMap, BTreeSet};

use crate::arity_adapter::RName;
use crate::model::{MethodDeclaration, PackageModel, ResolvedUsage};
use crate::r_parse::{BindingFact, BindingValue};
use crate::source::{SourceMap, Spanned};
use crate::tags::TagOrigin;

/// The three outcomes of automatic S3 export analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum S3ExportAnalysis {
    /// The object is known not to be an S3 method.
    OrdinaryExport,
    /// The object is a method of the selected local generic.
    S3Method { generic: String, class: String },
    /// Static facts are insufficient to decide whether the object is a method.
    Unresolved,
}

/// Applies the same proven S3 split used by NAMESPACE lowering to generated
/// Rd usage contributions. Explicit methods and non-generated usage remain
/// untouched.
pub(crate) fn classify_usage_methods(
    package: &mut PackageModel,
    sources: &SourceMap,
    provider: &dyn S3GenericProvider,
) {
    let mut analyzer = S3Analyzer::new(package, Some(sources), provider);
    let mut decisions = Vec::new();
    for (topic_key, topic) in &package.topics {
        for (index, contribution) in topic.usages.iter().enumerate() {
            if contribution.method.is_some()
                || !matches!(contribution.usage, ResolvedUsage::Generated(_))
            {
                continue;
            }
            let Some(object) = contribution.object.as_ref() else {
                continue;
            };
            let S3ExportAnalysis::S3Method { generic, class } = analyzer.analyze(object) else {
                continue;
            };
            decisions.push((
                topic_key.clone(),
                index,
                MethodDeclaration {
                    generic: Spanned::new(generic, contribution.block_span),
                    class: Spanned::new(class, contribution.block_span),
                    origin: TagOrigin::Implicit {
                        intro_span: contribution.block_span,
                    },
                },
            ));
        }
    }
    for (topic_key, index, method) in decisions {
        if let Some(topic) = package.topics.get_mut(&topic_key)
            && let Some(contribution) = topic.usages.get_mut(index)
            && contribution.method.is_none()
        {
            contribution.method = Some(method);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FunctionResolution {
    Function { calls_use_method: bool },
    NonFunction,
    Unknown,
}

/// Supplies positive facts about installed or otherwise known S3 generics.
///
/// The provider does not expose package layout or metadata representation;
/// callers may implement it with any source of facts. Local package bindings
/// always take precedence over this provider.
pub trait S3GenericProvider {
    fn is_s3_generic(&self, name: &str) -> bool;
}

/// An empty provider used by the default, package-local-only entry points.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyS3GenericProvider;

impl S3GenericProvider for EmptyS3GenericProvider {
    fn is_s3_generic(&self, _: &str) -> bool {
        false
    }
}

/// Resolves unique package-local binding facts and selects S3 method splits.
pub(super) struct S3Analyzer<'a, 'b, 'c> {
    facts: BTreeMap<String, Vec<&'a BindingFact>>,
    provider: &'c dyn S3GenericProvider,
    collate: bool,
    sources: Option<&'b SourceMap>,
    memo: BTreeMap<String, FunctionResolution>,
    visiting: BTreeSet<String>,
}

impl<'a, 'b, 'c> S3Analyzer<'a, 'b, 'c> {
    pub(super) fn new(
        package: &'a PackageModel,
        sources: Option<&'b SourceMap>,
        provider: &'c dyn S3GenericProvider,
    ) -> Self {
        let mut facts: BTreeMap<String, Vec<&'a BindingFact>> = BTreeMap::new();
        for fact in &package.bindings {
            facts
                .entry(fact.name.canonical.as_str().to_owned())
                .or_default()
                .push(fact);
        }
        Self {
            facts,
            provider,
            collate: package.collate,
            sources,
            memo: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    pub(super) fn resolve(&mut self, name: &str) -> FunctionResolution {
        if let Some(resolution) = self.memo.get(name) {
            return *resolution;
        }
        if !self.visiting.insert(name.to_owned()) {
            return FunctionResolution::Unknown;
        }

        let resolution = match self.facts.get(name).map(Vec::as_slice) {
            Some([fact]) => match &fact.value {
                BindingValue::Function { calls_use_method } => FunctionResolution::Function {
                    calls_use_method: *calls_use_method,
                },
                BindingValue::Alias(target) => {
                    match self.facts.get(target.as_str()).map(Vec::as_slice) {
                        Some([target_fact]) if self.target_precedes_alias(fact, target_fact) => {
                            self.resolve(target.as_str())
                        }
                        _ => FunctionResolution::Unknown,
                    }
                }
                BindingValue::NonFunction => FunctionResolution::NonFunction,
                BindingValue::S7Class(_) | BindingValue::S7Refused(_) | BindingValue::Unknown => {
                    FunctionResolution::Unknown
                }
            },
            Some(_) | None => FunctionResolution::Unknown,
        };
        self.visiting.remove(name);
        self.memo.insert(name.to_owned(), resolution);
        resolution
    }

    /// Whether the target assignment provably runs before the alias one.
    fn target_precedes_alias(&self, alias: &BindingFact, target: &BindingFact) -> bool {
        if alias.assignment_span.file == target.assignment_span.file {
            target.assignment_span.range.start() < alias.assignment_span.range.start()
        } else {
            !self.collate
                && self
                    .sources
                    .and_then(|sources| {
                        sources.compare_filename_order(
                            target.assignment_span.file,
                            alias.assignment_span.file,
                        )
                    })
                    .is_some_and(|ordering| ordering.is_lt())
        }
    }

    pub(super) fn is_proven_function(&mut self, name: &str) -> bool {
        matches!(self.resolve(name), FunctionResolution::Function { .. })
    }

    pub(super) fn analyze(&mut self, object: &RName) -> S3ExportAnalysis {
        let name = object.as_str();
        if !name.contains('.') {
            return S3ExportAnalysis::OrdinaryExport;
        }
        match self.resolve(name) {
            FunctionResolution::NonFunction => S3ExportAnalysis::OrdinaryExport,
            FunctionResolution::Function { .. } => self.select_split(name),
            FunctionResolution::Unknown => S3ExportAnalysis::Unresolved,
        }
    }

    fn select_split(&mut self, name: &str) -> S3ExportAnalysis {
        if let Some(class) = name.strip_prefix("all.equal.")
            && !class.is_empty()
            && self.is_s3_generic("all.equal")
        {
            return S3ExportAnalysis::S3Method {
                generic: "all.equal".to_owned(),
                class: class.to_owned(),
            };
        }
        for (dot, _) in name.match_indices('.') {
            let generic = &name[..dot];
            let class = &name[dot + 1..];
            if !generic.is_empty() && !class.is_empty() && self.is_s3_generic(generic) {
                return S3ExportAnalysis::S3Method {
                    generic: generic.to_owned(),
                    class: class.to_owned(),
                };
            }
        }
        S3ExportAnalysis::Unresolved
    }
}

impl S3Analyzer<'_, '_, '_> {
    fn is_s3_generic(&mut self, name: &str) -> bool {
        // A package-local binding shadows installed generic metadata,
        // including unresolved and ambiguous local facts.
        if self.facts.contains_key(name) {
            return matches!(
                self.resolve(name),
                FunctionResolution::Function {
                    calls_use_method: true
                }
            );
        }
        self.provider.is_s3_generic(name)
    }
}
