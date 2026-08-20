use crate::model::{PackageModel, TopicKey};
use crate::tags::TopicRef;

use super::merge::project_local_topic;
use super::types::InheritableTopic;

/// A normalized provider lookup request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicRequest {
    /// A topic in the package currently being documented.
    Local { topic: TopicRef },
    /// A topic supplied by another package.
    External { package: String, topic: TopicRef },
}

/// Canonical identity used for memoization and provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocumentationIdentity {
    /// A local topic key.
    Local(TopicKey),
    /// An external package/topic pair.
    External { package: String, topic: String },
}

/// Whether a provider could confirm an exact documentation alias lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicExistence {
    /// The lookup completed and the alias was present or absent.
    Known(bool),
    /// The provider could not complete the lookup.
    Unavailable,
}

/// Storage-independent provider failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentationError {
    /// Stable failure classification.
    pub kind: DocumentationErrorKind,
    /// Package involved in the lookup, if known.
    pub package: Option<String>,
    /// Topic involved in the lookup, if known.
    pub topic: Option<String>,
    /// Human-readable provider detail.
    pub detail: String,
}

/// Stable classes of documentation-provider failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentationErrorKind {
    /// The package name was not valid for a provider.
    InvalidPackageName,
    /// The package could not be located.
    PackageUnavailable,
    /// Package help metadata could not be read.
    HelpDatabaseUnreadable,
    /// The alias index could not be read.
    AliasIndexUnreadable,
    /// The requested topic could not be read.
    TopicUnreadable,
    /// External Rd could not be converted to the shared view.
    RdLoweringFailed,
}

/// The provider boundary deliberately contains no storage-specific types.
pub trait DocumentationProvider {
    /// Returns a projected topic, `None` for a clean miss, or a classified
    /// failure when lookup could not be completed.
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError>;

    /// Checks for an exact alias in an external package's help index.
    ///
    /// Providers that cannot perform this query leave inherited links
    /// unchanged by returning the default unavailable state.
    fn topic_exists(&self, _package: &str, _alias: &str) -> TopicExistence {
        TopicExistence::Unavailable
    }
}

pub(super) fn lookup_local_topic(
    package: &PackageModel,
    requested: &str,
) -> Result<TopicKey, LocalLookupError> {
    let exact = TopicKey(requested.to_owned());
    if package.topics.contains_key(&exact) {
        return Ok(exact);
    }
    let matches = package
        .topics
        .iter()
        .filter(|(_, topic)| topic.aliases.iter().any(|alias| alias.name.0 == requested))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [key] => Ok(key.clone()),
        [] => Err(LocalLookupError::Missing),
        _ => Err(LocalLookupError::Ambiguous),
    }
}

/// Provider for package-local topics with deterministic exact/alias lookup.
pub struct LocalDocumentationProvider<'a> {
    package: &'a PackageModel,
}

impl<'a> LocalDocumentationProvider<'a> {
    /// Creates a provider over an immutable package model.
    #[must_use]
    pub const fn new(package: &'a PackageModel) -> Self {
        Self { package }
    }
}

impl DocumentationProvider for LocalDocumentationProvider<'_> {
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        let TopicRequest::Local { topic } = request else {
            return Ok(None);
        };
        let Ok(key) = lookup_local_topic(self.package, &topic.0) else {
            return Ok(None);
        };
        let found = self
            .package
            .topics
            .get(&key)
            .expect("a successful local lookup has a topic");
        Ok(Some(project_local_topic(&key, found)))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalLookupError {
    Missing,
    Ambiguous,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DefaultProvider;

    impl DocumentationProvider for DefaultProvider {
        fn get_topic(
            &self,
            _request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            Ok(None)
        }
    }

    #[test]
    fn topic_existence_defaults_to_unavailable() {
        assert_eq!(
            DefaultProvider.topic_exists("package", "alias"),
            TopicExistence::Unavailable
        );
    }
}
