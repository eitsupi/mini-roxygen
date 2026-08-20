//! Lowers namespace requests into deterministic, in-memory NAMESPACE text.
//!
//! The lowering pipeline is deliberately independent of Rd generation:
//! collect, validate, normalize, deduplicate, merge, sort, and render. Raw
//! [`crate::tags::NamespaceTag`] values do not cross the validation boundary.

mod ir;
mod pipeline;
mod render;
mod s3;

#[cfg(test)]
mod tests;

pub use ir::NamespaceBuildOutput;
pub(crate) use pipeline::build_namespace_with_sources_and_provider;
#[cfg(test)]
pub(crate) use pipeline::{
    build_namespace, build_namespace_with_provider, build_namespace_with_sources,
};
pub(crate) use s3::classify_usage_methods;
pub use s3::{EmptyS3GenericProvider, S3GenericProvider};
