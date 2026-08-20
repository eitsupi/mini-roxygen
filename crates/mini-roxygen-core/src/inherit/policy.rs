use crate::diagnostic::Diagnostics;

use super::types::ResolvedPackageModel;

/// Controls whether the resolver may consult an external provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalInheritancePolicy {
    /// Do not consult the provider at all.
    Off,
    /// Keep local output and warn when external lookup fails.
    BestEffort,
    /// Keep partial output but report external lookup failures as errors.
    Strict,
}

/// Whether an `Off` policy was explicitly selected or inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPolicySource {
    /// The caller explicitly selected `Off`.
    Explicit,
    /// No external library path/configuration was supplied.
    NoConfiguredLibrary,
}

/// Resolver configuration. The source is retained so hermetic diagnostics can
/// distinguish an explicit opt-out from the default without a CLI dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InheritanceOptions {
    /// External lookup policy.
    pub external: ExternalInheritancePolicy,
    /// Meaningful for `Off`; retained for all modes to keep construction plain.
    pub external_source: ExternalPolicySource,
}

impl Default for InheritanceOptions {
    fn default() -> Self {
        Self {
            external: ExternalInheritancePolicy::Off,
            external_source: ExternalPolicySource::NoConfiguredLibrary,
        }
    }
}

/// Result of resolving one package.
#[derive(Debug, Clone, PartialEq)]
pub struct InheritanceOutput {
    /// The resolved package model.
    pub package: ResolvedPackageModel,
    /// Resolution diagnostics, including recoverable provider failures.
    pub diagnostics: Diagnostics,
}
