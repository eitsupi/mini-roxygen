//! Resolution of `@inherit` and `@inheritParams` requests.
//!
//! The model builder intentionally keeps requests in
//! [`PackageModel`](crate::model::PackageModel).  This
//! module projects topics into an inheritable view, resolves that view, and
//! returns a separate model which the Rd builder can accept without a runtime
//! unresolved-inheritance check.

mod external;
mod merge;
mod policy;
mod provider;
mod resolver;
mod types;

mod graph;
mod selection;

#[cfg(test)]
mod tests;

pub use external::project_external_topic;
pub use policy::{ExternalInheritancePolicy, ExternalPolicySource, InheritanceOptions};
pub(crate) use provider::LocalDocumentationProvider;
pub use provider::{
    DocumentationError, DocumentationErrorKind, DocumentationIdentity, DocumentationProvider,
    TopicExistence, TopicRequest,
};
pub(crate) use resolver::resolve_inheritance_with_substitutions;

#[cfg(test)]
pub(crate) use resolver::resolve_inheritance;
pub use types::{
    DocumentationOrigin, InheritableContent, InheritableFields, InheritableParamGroup,
    InheritableParamLabel, InheritableSection, InheritableTopic, InheritanceTrace, ResolvedContent,
    ResolvedPackageModel, ResolvedRdTopic,
};
