//! Defines the structured diagnostics shared by parsing and output layers.
//!
//! Diagnostics retain source provenance so warnings and errors can identify
//! the relevant file and byte span without requiring evaluation of R code.
//! Context is represented as arbitrary string key-value pairs. This keeps the
//! diagnostic contract extensible for tag, topic, and provider information
//! without prematurely coupling it to context types owned by later layers.

use crate::source::Span;

/// The impact level of a diagnostic.
/// Severity ordering is intentionally undefined; if ordering becomes necessary, it should be introduced with its meaning made explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Prevents successful generation.
    Error,
    /// Reports a problem while allowing generation to continue where possible.
    Warning,
    /// Provides non-problem information.
    Info,
}

macro_rules! define_diagnostic_codes {
    (
        $(
            $(#[$variant_doc:meta])*
            $variant:ident, $code:literal, $severity:ident
        ),+ $(,)?
    ) => {
        /// A stable diagnostic identifier.
        ///
        /// Codes are closed and use lowercase kebab-case strings so they remain
        /// suitable for machine-readable output and CLI filtering.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum DiagnosticCode {
            $(
                $(#[$variant_doc])*
                $variant,
            )+
        }

        impl DiagnosticCode {
            /// All diagnostic codes, in declaration order.
            ///
            /// This public list makes the closed code set available to exhaustive
            /// renderers and validation without requiring callers to duplicate it.
            /// Its exhaustiveness is structurally guaranteed because the list is
            /// generated from the same table as the enum.
            pub const ALL: &[Self] = &[
                $(Self::$variant,)+
            ];

            /// Returns the stable lowercase kebab-case code string.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// Returns the default severity for this code.
            ///
            /// Unknown tags and ambiguous external aliases are warnings as specified
            /// by the diagnostic design. Other codes default to errors because they
            /// indicate unsupported or invalid input and have no design-specified
            /// recoverable severity. `@noMd` uses [`Self::UnsupportedTag`], while
            /// `@md` is accepted as a redundant marker because Markdown is always on.
            #[must_use]
            pub const fn default_severity(self) -> Severity {
                match self {
                    $(Self::$variant => Severity::$severity,)+
                }
            }
        }
    };
}

define_diagnostic_codes! {
    /// An R source syntax error.
    RSyntaxError, "r-syntax-error", Error,
    /// A roxygen tag could not be parsed.
    TagParseError, "tag-parse-error", Error,
    /// A tag was repeated where repetition is invalid.
    DuplicateTag, "duplicate-tag", Error,
    /// An identical inheritance request was repeated after topic merging.
    DuplicateInheritanceRequest, "duplicate-inheritance-request", Warning,
    /// A documented block has no name from which a topic can be built, so an
    /// Rd identity cannot be assigned without guessing.
    MissingTopicIdentity, "missing-topic-identity", Error,
    /// A data-object string literal decoded to an empty name.
    EmptyDataName, "empty-data-name", Error,
    /// A data-object string literal needs unsupported escape decoding.
    UndecodableDataName, "undecodable-data-name", Error,
    /// Package-level documentation was recognised but has no implementation yet.
    PackageDocumentationUnsupported, "package-documentation-unsupported", Error,
    /// Authors@R could not be statically parsed for a package topic.
    PackageAuthorsParse, "package-authors-parse", Warning,
    /// A package topic has no title from any local, metadata, or inherited source.
    MissingPackageTitle, "missing-package-title", Error,
    /// DESCRIPTION has no Description field for a package topic.
    MissingPackageDescription, "missing-package-description", Warning,
    /// A data topic has no statically supplied format description.
    MissingDataFormat, "missing-data-format", Warning,
    /// A parameter name received more than one description, which would make
    /// the generated argument documentation ambiguous.
    ConflictingParamDescription, "conflicting-param-description", Error,
    /// A section title received more than one body, which would make the
    /// resulting named section ambiguous.
    ConflictingSectionTitle, "conflicting-section-title", Error,
    /// An alias was claimed by more than one documentation topic.
    ConflictingAlias, "conflicting-alias", Error,
    /// A function's statically extracted signature could not become usage text,
    /// so silently omitting the usage would hide a source-level failure.
    UsageGenerationFailed, "usage-generation-failed", Error,
    /// A documented S7 class uses metadata outside the static subset.
    UnsupportedS7Constructor, "unsupported-s7-constructor", Error,
    /// A tag is not known to mini-roxygen.
    UnknownTag, "unknown-tag", Warning,
    /// An @examplesIf condition is not one syntactically valid R expression.
    InvalidExamplesIfCondition, "invalid-examples-if-condition", Error,
    /// An @examplesIf tag has no example source after its condition.
    EmptyExamplesIfBody, "empty-examples-if-body", Error,
    /// A known tag is not supported by mini-roxygen.
    UnsupportedTag, "unsupported-tag", Error,
    /// An inheritance target could not be resolved.
    UnresolvedInherit, "unresolved-inherit", Error,
    /// An inheritance cycle was detected.
    InheritCycle, "inherit-cycle", Error,
    /// A requested inherited section was not present in the donor topic.
    MissingInheritedSection, "missing-inherited-section", Error,
    /// A donor topic contained more than one matching inherited section.
    AmbiguousInheritedSection, "ambiguous-inherited-section", Error,
    /// External inheritance was skipped by the configured policy.
    ExternalInheritanceDisabled, "external-inheritance-disabled", Warning,
    /// A documented parameter is missing.
    MissingParam, "missing-param", Warning,
    /// An inherit-parameter selection is invalid.
    InvalidSelection, "invalid-selection", Error,
    /// An inherit-parameter selector uses syntax outside the supported grammar.
    UnsupportedSelection, "unsupported-selection", Error,
    /// An S3 generic and class could not be selected unambiguously.
    AmbiguousS3, "ambiguous-s3", Error,
    /// A bare export could not be classified as an ordinary export or S3 method.
    UnresolvedS3Export, "unresolved-s3-export", Warning,
    /// An S3 method declaration conflicts with another block in the topic.
    DuplicateMethod, "duplicate-method", Error,
    /// A configured S3 registrar call has malformed static arguments.
    InvalidS3Registration, "invalid-s3-registration", Error,
    /// A configured S3 registrar call has runtime-computed generic or class
    /// values, so static registration metadata was not generated.
    DynamicS3Registration, "dynamic-s3-registration", Info,
    /// Multiple registrar facts claim one documented method target.
    AmbiguousS3Registration, "ambiguous-s3-registration", Error,
    /// A documented registered S3 method has no export or load-time suppression.
    UnexportedS3Method, "unexported-s3-method", Warning,
    /// A NULL S3 export tag has no statically known registration metadata.
    UnresolvedS3MethodMetadata, "unresolved-s3-method-metadata", Error,
    /// An unmanaged generated output would be overwritten.
    UnmanagedOutputOverwrite, "unmanaged-output-overwrite", Error,
    /// A NAMESPACE directive is invalid.
    InvalidNamespaceDirective, "invalid-namespace-directive", Error,
    /// A namespace directive imports from the package being documented, which
    /// asks R for nothing and is more often a misspelled neighbour.
    SelfImport, "self-import", Warning,
    /// A raw Rd macro is outside the supported set.
    UnsupportedRawRdMacro, "unsupported-raw-rd-macro", Error,
    /// A recognized raw Rd macro's brace never closes.
    UnterminatedRawRdMacro, "unterminated-raw-rd-macro", Error,
    /// Inline R requiring evaluation was encountered.
    UnsupportedInlineR, "unsupported-inline-r", Error,
    /// Inline R had no exact static substitution.
    UndefinedInlineRSubstitution, "undefined-inline-r-substitution", Error,
    /// A configured inline R replacement was not valid Rd.
    InvalidInlineRSubstitution, "invalid-inline-r-substitution", Error,
    /// A user substitution did not match any source expression.
    UnusedInlineRSubstitution, "unused-inline-r-substitution", Warning,
    /// An external alias has multiple possible resolutions.
    AmbiguousExternalAlias, "ambiguous-external-alias", Warning,
    /// The two parser views of Markdown structure disagree.
    MarkdownStructureMismatch, "markdown-structure-mismatch", Error,
    /// A Markdown construct is not supported by the current conversion step.
    UnsupportedMarkdownConstruct, "unsupported-markdown-construct", Error,
    /// A level-1 Markdown heading is flattened because it has no structural
    /// lowering in the current conversion subset.
    UnsupportedMarkdownHeading, "unsupported-markdown-heading", Warning,
    /// A roxygen line is not attached to any top-level expression window.
    UnattachedRoxygenBlock, "unattached-roxygen-block", Warning,
    /// A topic has no title and cannot form a valid Rd document.
    MissingTopicTitle, "missing-topic-title", Error,
    /// Package and data contributions were merged under one topic key.
    ConflictingTopicKind, "conflicting-topic-kind", Error,
    /// Two topics would write the same normalized Rd filename.
    ConflictingRdFileName, "conflicting-rd-file-name", Error,
    /// A topic name normalizes to an empty Rd filename.
    UnnameableRdFile, "unnameable-rd-file", Error,
    /// An Rd AST could not be serialized.
    RdSerializationFailed, "rd-serialization-failed", Error,
}

/// A source label attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Label {
    /// The source range highlighted by the label.
    pub span: Span,
    /// Explains what the highlighted range means.
    pub message: String,
}

impl Label {
    /// Creates a label from a span and explanation.
    #[must_use]
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// A structured source-aware diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The diagnostic's impact level.
    pub severity: Severity,
    /// The stable machine-readable identifier.
    pub code: DiagnosticCode,
    /// The diagnostic heading.
    pub message: String,
    /// The required primary problem label.
    pub primary: Label,
    /// Related source labels.
    pub secondary: Vec<Label>,
    /// An optional suggested action.
    pub help: Option<String>,
    /// Extensible context such as tag, topic, or provider values.
    pub context: Vec<(String, String)>,
}

impl Diagnostic {
    /// Creates a diagnostic with all required fields.
    #[must_use]
    pub fn new(
        severity: Severity,
        code: DiagnosticCode,
        message: impl Into<String>,
        primary: Label,
    ) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            primary,
            secondary: Vec::new(),
            help: None,
            context: Vec::new(),
        }
    }

    /// Adds one related label.
    #[must_use]
    pub fn with_secondary(mut self, label: Label) -> Self {
        self.secondary.push(label);
        self
    }

    /// Adds related labels.
    #[must_use]
    pub fn with_secondaries(mut self, labels: impl IntoIterator<Item = Label>) -> Self {
        self.secondary.extend(labels);
        self
    }

    /// Adds a suggested action.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Adds an extensible context entry, such as a tag, topic, or provider.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.push((key.into(), value.into()));
        self
    }
}

/// A collection that accumulates diagnostics across all topics.
///
/// Callers can continue pushing diagnostics after an error; generation is not
/// required to stop at the first error. The final status can be made non-zero
/// by checking [`Diagnostics::has_errors`] after all topics have been visited.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    diagnostics: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Creates an empty diagnostic collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    /// Appends a diagnostic without stopping collection.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Returns whether at least one error has been collected.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// Returns the number of collected diagnostics.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.diagnostics.len()
    }

    /// Returns whether no diagnostics have been collected.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Iterates over diagnostics in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
    use crate::source::{FileId, Span, TextRange};

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId::new(0), TextRange::new(start, end))
    }

    #[test]
    fn diagnostic_builder_sets_optional_fields() {
        let primary = Label::new(span(0, 3), "primary");
        let secondary = Label::new(span(5, 8), "related");
        let diagnostic = Diagnostic::new(
            Severity::Warning,
            DiagnosticCode::UnknownTag,
            "unknown tag",
            primary.clone(),
        )
        .with_secondary(secondary.clone())
        .with_help("remove the tag")
        .with_context("tag", "unknown")
        .with_context("topic", "example");

        assert_eq!(diagnostic.primary, primary);
        assert_eq!(diagnostic.secondary, vec![secondary]);
        assert_eq!(diagnostic.help.as_deref(), Some("remove the tag"));
        assert_eq!(
            diagnostic.context,
            vec![
                ("tag".to_owned(), "unknown".to_owned()),
                ("topic".to_owned(), "example".to_owned()),
            ]
        );
    }

    #[test]
    fn diagnostic_codes_are_unique() {
        let codes = DiagnosticCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(codes.len(), DiagnosticCode::ALL.len());
    }

    #[test]
    fn every_diagnostic_code_has_a_default_severity() {
        for code in DiagnosticCode::ALL {
            let _ = code.default_severity();
        }
        assert_eq!(
            DiagnosticCode::UnknownTag.default_severity(),
            Severity::Warning
        );
        assert_eq!(
            DiagnosticCode::AmbiguousExternalAlias.default_severity(),
            Severity::Warning
        );
        assert_eq!(
            DiagnosticCode::UnsupportedRawRdMacro.default_severity(),
            Severity::Error
        );
        assert_eq!(
            DiagnosticCode::UnterminatedRawRdMacro.default_severity(),
            Severity::Error
        );
        assert_eq!(
            DiagnosticCode::DynamicS3Registration.default_severity(),
            Severity::Info
        );
        assert_eq!(
            DiagnosticCode::UnsupportedMarkdownHeading.default_severity(),
            Severity::Warning
        );
    }

    #[test]
    fn diagnostics_report_errors_without_stopping_collection() {
        let primary = Label::new(span(0, 1), "problem");
        let mut diagnostics = Diagnostics::new();

        diagnostics.push(Diagnostic::new(
            Severity::Error,
            DiagnosticCode::TagParseError,
            "bad tag",
            primary.clone(),
        ));
        diagnostics.push(Diagnostic::new(
            Severity::Info,
            DiagnosticCode::MarkdownStructureMismatch,
            "additional information",
            primary,
        ));

        assert!(diagnostics.has_errors());
        assert_eq!(diagnostics.len(), 2);
        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics.iter().count(), 2);
    }

    #[test]
    fn diagnostics_without_errors_are_not_errors() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(Diagnostic::new(
            Severity::Warning,
            DiagnosticCode::UnknownTag,
            "unknown tag",
            Label::new(span(0, 1), "tag"),
        ));

        assert!(!diagnostics.has_errors());
    }

    #[test]
    fn dynamic_s3_diagnostics_are_informational() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(Diagnostic::new(
            Severity::Info,
            DiagnosticCode::DynamicS3Registration,
            "dynamic S3 registration is delegated to runtime",
            Label::new(span(0, 1), "dynamic registrar call"),
        ));

        assert!(!diagnostics.has_errors());
    }
}
