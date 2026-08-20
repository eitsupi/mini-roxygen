//! Resolves implicit topic identities and identity-related diagnostics.
//!
//! Object association has distinct source-span and refusal rules, so it is
//! isolated from merge mechanics while retaining the same recovery behavior.

use crate::arity_adapter::RName;
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::r_parse::{
    AssociationRefusal, BlockTarget, DataObject, FunctionObject, PackageSentinel,
};
use crate::source::Span;

use super::DocumentedBlock;

pub(in crate::model) fn implicit_object_name(target: &BlockTarget) -> Option<&RName> {
    match target {
        BlockTarget::FunctionAssignment(FunctionObject { name, .. })
        | BlockTarget::ValueAssignment(crate::r_parse::ValueObject { name, .. }) => {
            Some(&name.canonical)
        }
        BlockTarget::Null { .. }
        | BlockTarget::DataObject(_)
        | BlockTarget::PackageDocumentation(_)
        | BlockTarget::Call(_)
        | BlockTarget::Refused(_) => None,
    }
}

pub(in crate::model) fn data_object_name(
    target: &BlockTarget,
) -> Option<&crate::r_parse::DataName> {
    match target {
        BlockTarget::DataObject(DataObject { name }) => Some(&name.value),
        _ => None,
    }
}

pub(in crate::model) fn implicit_object_span(target: &BlockTarget) -> Option<Span> {
    match target {
        BlockTarget::FunctionAssignment(FunctionObject { name, .. })
        | BlockTarget::ValueAssignment(crate::r_parse::ValueObject { name, .. }) => {
            Some(name.spelling)
        }
        BlockTarget::Null { .. }
        | BlockTarget::DataObject(_)
        | BlockTarget::PackageDocumentation(_)
        | BlockTarget::Call(_)
        | BlockTarget::Refused(_) => None,
    }
}

pub(in crate::model) fn data_object_span(target: &BlockTarget) -> Option<Span> {
    match target {
        BlockTarget::DataObject(DataObject { name }) => Some(name.span),
        _ => None,
    }
}

pub(in crate::model) fn is_refused_or_null(target: &BlockTarget) -> bool {
    matches!(target, BlockTarget::Null { .. } | BlockTarget::Refused(_))
}

pub(in crate::model) fn emit_missing_identity(
    diagnostics: &mut Diagnostics,
    block: &DocumentedBlock,
) {
    let (primary_span, primary_message) = match &block.target {
        BlockTarget::Null { span } => (*span, "documented NULL has no topic name"),
        BlockTarget::Refused(refusal) => {
            (refusal_span(refusal), "refused target has no topic name")
        }
        _ => (block.block_span, "documented block has no topic name"),
    };
    let diagnostic = Diagnostic::new(
        Severity::Error,
        DiagnosticCode::MissingTopicIdentity,
        primary_message,
        Label::new(primary_span, primary_message),
    )
    .with_secondary(Label::new(block.block_span, "documentation block"))
    .with_help("add an explicit @name or suppress this block with @noRd");
    diagnostics.push(diagnostic);
}

pub(in crate::model) fn refusal_span(refusal: &AssociationRefusal) -> Span {
    match refusal {
        AssociationRefusal::CompoundAssignment { target_span }
        | AssociationRefusal::InvalidAssignmentTarget { target_span }
        | AssociationRefusal::UndecodableBinding { target_span, .. } => *target_span,
        AssociationRefusal::UndecodableDataName { span, .. }
        | AssociationRefusal::EmptyDataName { span } => *span,
        AssociationRefusal::UnsupportedExpression { span } => *span,
    }
}

pub(in crate::model) fn emit_package_documentation_diagnostic(
    diagnostics: &mut Diagnostics,
    sentinel: PackageSentinel,
) {
    diagnostics.push(
        Diagnostic::new(
            Severity::Error,
            DiagnosticCode::PackageDocumentationUnsupported,
            "package-level documentation is recognised but not yet implemented",
            Label::new(
                sentinel.span,
                "package-level documentation is not implemented",
            ),
        )
        .with_help(
            "use an object-level documentation block until package documentation is implemented",
        ),
    );
}

pub(in crate::model) fn emit_data_name_diagnostic(
    diagnostics: &mut Diagnostics,
    refusal: &AssociationRefusal,
) {
    let (span, code, message) = match refusal {
        AssociationRefusal::EmptyDataName { span } => (
            *span,
            DiagnosticCode::EmptyDataName,
            "data object name is empty",
        ),
        AssociationRefusal::UndecodableDataName { span, reason } => (
            *span,
            DiagnosticCode::UndecodableDataName,
            match reason {
                crate::arity_adapter::RNameDecodeError::ContainsBackslash => {
                    "data object name contains an unsupported escape"
                }
                crate::arity_adapter::RNameDecodeError::InvalidSpelling => {
                    "data object name has invalid string spelling"
                }
                crate::arity_adapter::RNameDecodeError::EmptyName => "data object name is empty",
                crate::arity_adapter::RNameDecodeError::MixedUnicodeAndByteEscapes => {
                    "data object name mixes Unicode with hex or octal escapes"
                }
                crate::arity_adapter::RNameDecodeError::NulCharacter => {
                    "data object name contains a nul character"
                }
            },
        ),
        _ => return,
    };
    diagnostics.push(Diagnostic::new(
        Severity::Error,
        code,
        message,
        Label::new(span, message),
    ));
}

pub(in crate::model) fn origin_span(origin: &crate::tags::TagOrigin) -> Span {
    match origin {
        crate::tags::TagOrigin::Explicit { full_span, .. } => *full_span,
        crate::tags::TagOrigin::Implicit { intro_span } => *intro_span,
    }
}

#[cfg(test)]
mod tests {
    use crate::model::TopicKey;
    use crate::model::test_support::model;

    #[test]
    fn data_name_refusals_and_package_documentation_have_distinct_outcomes() {
        let named_data = model(
            r#"#' @name manual
"dataset name"
"#,
        );
        let named_topic = &named_data.package.topics[&TopicKey("manual".into())];
        assert_eq!(
            named_topic
                .aliases
                .iter()
                .map(|alias| alias.name.0.as_str())
                .collect::<Vec<_>>(),
            ["manual", "dataset name"]
        );
        assert!(named_data.diagnostics.is_empty());

        let undecodable = model("#' documented\n\"a\\x2e b\"\n");
        assert!(undecodable.package.topics.is_empty());
        assert!(undecodable.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::UndecodableDataName
        }));
        assert!(!undecodable.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::MissingTopicIdentity
        }));

        let empty = model(
            r#"#' documented
""
"#,
        );
        assert!(empty.package.topics.is_empty());
        assert!(empty.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::EmptyDataName
        }));

        let package = model(
            r#"#' package
"_PACKAGE"
"#,
        );
        assert!(package.package.topics.is_empty());
        assert!(package.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::PackageDocumentationUnsupported
        }));

        let silent = model(
            r#"#' @noRd
"_PACKAGE"
"#,
        );
        assert!(silent.package.topics.is_empty());
        assert!(silent.diagnostics.is_empty());
    }

    #[test]
    fn null_and_refused_targets_need_name_but_explicit_name_recovers() {
        let missing = model(
            r#"#' documented
NULL
"#,
        );
        assert!(
            missing.diagnostics.iter().any(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingTopicIdentity)
        );

        let recovered = model(
            r#"#' @name manual
NULL
"#,
        );
        assert!(recovered.diagnostics.is_empty());
        assert!(
            recovered
                .package
                .topics
                .contains_key(&TopicKey("manual".into()))
        );
        assert_eq!(
            recovered.package.topics[&TopicKey("manual".into())].aliases[0]
                .name
                .0,
            "manual"
        );

        let refused = model(
            r#"#' documented
x$y <- 1
"#,
        );
        assert!(
            refused.diagnostics.iter().any(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingTopicIdentity)
        );
        let recovered = model(
            r#"#' @name manual
x$y <- 1
"#,
        );
        assert!(recovered.diagnostics.is_empty());
        assert_eq!(
            recovered.package.topics[&TopicKey("manual".into())].aliases[0]
                .name
                .0,
            "manual"
        );
    }

    #[test]
    fn every_association_refusal_is_recoverable_only_by_name() {
        let cases = ["x$y <- 1", "NULL <- 1", r#""a\x2e b" <- 1"#, "1 + 2"];
        for expression in cases {
            let missing = model(&format!(
                r#"#' documented
{expression}
"#
            ));
            assert!(missing.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == crate::diagnostic::DiagnosticCode::MissingTopicIdentity
            }));

            let recovered = model(&format!(
                r#"#' @name manual
{expression}
"#
            ));
            assert!(recovered.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != crate::diagnostic::DiagnosticCode::MissingTopicIdentity
            }));
            assert!(
                recovered
                    .package
                    .topics
                    .contains_key(&TopicKey("manual".into()))
            );
            assert_eq!(
                recovered.package.topics[&TopicKey("manual".into())].aliases[0]
                    .name
                    .0,
                "manual"
            );
        }
    }
}
