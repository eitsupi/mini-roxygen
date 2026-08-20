//! Combines parsed tags and static object facts into topic and namespace IR.
//!
//! The model is divided by responsibility so its public intermediate
//! representation remains stable while merge, identity, slot, usage, and tag
//! lookup rules can stay local to the work they perform.

mod identity;
mod ir;
mod lookup;
mod merge;
mod slots;
mod usage;

#[cfg(test)]
pub(crate) mod test_support;

pub use ir::TopicKey;
pub(crate) use ir::TopicKindOrigin;
pub(crate) use ir::{
    Alias, BlockRef, DocumentedBlock, FormalContribution, FormalName, FormalNames,
    InheritanceRequest, MethodDeclaration, ModelOutput, NamedSection, NamespaceRequest,
    PackageAuthor, PackageComment, PackageIdentity, PackageLink, PackageMetadataDiagnosticState,
    PackageModel, PackagePerson, PackageSeeAlso, ParamDescription, RdTopic, RdTopicKind,
    ResolvedUsage, UsageContribution,
};
pub(crate) use merge::build_package_model_with_metadata_bindings_and_registrations;

#[cfg(test)]
pub(crate) use merge::{
    build_package_model, build_package_model_with_bindings, build_package_model_with_metadata,
    build_package_model_with_metadata_and_bindings,
};

pub(in crate::model) use identity::{
    data_object_name, data_object_span, emit_data_name_diagnostic, emit_missing_identity,
    emit_package_documentation_diagnostic, implicit_object_name, implicit_object_span,
    is_refused_or_null, origin_span,
};
pub(in crate::model) use lookup::{
    first_name, first_order, first_rdname, has_no_rd, suppresses_default_aliases,
};
pub(in crate::model) use slots::{
    DuplicateSlotKind, emit_duplicate, emit_duplicate_method, set_field, set_tag,
};
pub(in crate::model) use usage::{resolve_explicit_usage, resolve_formal_names, resolve_usage};
