//! The closed semantic tag registry.
//!
//! Keeping syntax policy in one table makes recognition, value validation, and
//! multiline behavior auditable without coupling those concerns to payload
//! construction.

/// The policy applied to a normalized tag value that contains newlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Multiline {
    /// Report a diagnostic but retain the complete value.
    Never,
    /// Retain only immediately following, more-indented content.
    Indent,
    /// Retain unrestricted multiline content.
    Always,
}

/// Whether a tag may contain a normalized value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueRequirement {
    /// An empty normalized value is invalid.
    Required,
    /// An empty normalized value is valid.
    Allowed,
    /// A non-empty normalized value is invalid.
    Forbidden,
}

/// The payload grammar selected after registry validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueGrammar {
    Markdown,
    Plain,
    Words,
    Code,
    RCode,
    Param,
    Inherit,
    InheritParams,
    Section,
    Toggle,
    Marker,
}

/// The semantic payload kind selected by a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownTagKind {
    Title,
    Description,
    Details,
    Return,
    SeeAlso,
    References,
    Note,
    Format,
    Source,
    Author,
    Param,
    Name,
    RdName,
    Aliases,
    Keywords,
    NoRd,
    Examples,
    ExamplesIf,
    Usage,
    Method,
    Order,
    Inherit,
    InheritSection,
    InheritParams,
    Section,
    Namespace(NamespaceTagKind),
    Unsupported(UnsupportedTagKind),
    MarkdownMarker,
    NoMd,
}

/// A recognized NAMESPACE tag whose value is intentionally still opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamespaceTagKind {
    Export,
    ExportS3Method,
    Import,
    ImportFrom,
    RawNamespace,
    UseDynLib,
    ExportPattern,
    ExportClass,
    ExportMethod,
    ImportClassesFrom,
    ImportMethodsFrom,
}

/// A recognized tag that cannot be lowered without evaluating R.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsupportedTagKind {
    Eval,
    EvalRd,
    EvalNamespace,
    Template,
    TemplateVar,
    IncludeRmd,
}

/// One complete semantic policy for one tag spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TagSpec {
    pub(super) name: &'static str,
    pub(super) kind: KnownTagKind,
    pub(super) grammar: ValueGrammar,
    pub(super) requirement: ValueRequirement,
    pub(super) multiline: Multiline,
}

const TAG_SPECS: &[TagSpec] = &[
    TagSpec {
        name: "title",
        kind: KnownTagKind::Title,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "description",
        kind: KnownTagKind::Description,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "details",
        kind: KnownTagKind::Details,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "return",
        kind: KnownTagKind::Return,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "returns",
        kind: KnownTagKind::Return,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "seealso",
        kind: KnownTagKind::SeeAlso,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "references",
        kind: KnownTagKind::References,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "note",
        kind: KnownTagKind::Note,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "format",
        kind: KnownTagKind::Format,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "source",
        kind: KnownTagKind::Source,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "author",
        kind: KnownTagKind::Author,
        grammar: ValueGrammar::Markdown,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "param",
        kind: KnownTagKind::Param,
        grammar: ValueGrammar::Param,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "name",
        kind: KnownTagKind::Name,
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "rdname",
        kind: KnownTagKind::RdName,
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "aliases",
        kind: KnownTagKind::Aliases,
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "keywords",
        kind: KnownTagKind::Keywords,
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "noRd",
        kind: KnownTagKind::NoRd,
        grammar: ValueGrammar::Toggle,
        requirement: ValueRequirement::Forbidden,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "examples",
        kind: KnownTagKind::Examples,
        grammar: ValueGrammar::RCode,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "examplesIf",
        kind: KnownTagKind::ExamplesIf,
        grammar: ValueGrammar::RCode,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "usage",
        kind: KnownTagKind::Usage,
        grammar: ValueGrammar::RCode,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "inherit",
        kind: KnownTagKind::Inherit,
        grammar: ValueGrammar::Inherit,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "inheritSection",
        kind: KnownTagKind::InheritSection,
        grammar: ValueGrammar::Inherit,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "inheritParams",
        kind: KnownTagKind::InheritParams,
        grammar: ValueGrammar::InheritParams,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "section",
        kind: KnownTagKind::Section,
        grammar: ValueGrammar::Section,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "export",
        kind: KnownTagKind::Namespace(NamespaceTagKind::Export),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "exportS3Method",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ExportS3Method),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "method",
        kind: KnownTagKind::Method,
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "order",
        kind: KnownTagKind::Order,
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "import",
        kind: KnownTagKind::Namespace(NamespaceTagKind::Import),
        grammar: ValueGrammar::Words,
        // Namespace lowering owns arity validation so malformed values can
        // produce InvalidNamespaceDirective at the namespace boundary.
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "importFrom",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ImportFrom),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Indent,
    },
    TagSpec {
        name: "rawNamespace",
        kind: KnownTagKind::Namespace(NamespaceTagKind::RawNamespace),
        grammar: ValueGrammar::Code,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "useDynLib",
        kind: KnownTagKind::Namespace(NamespaceTagKind::UseDynLib),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "exportPattern",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ExportPattern),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "exportClass",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ExportClass),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "exportMethod",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ExportMethod),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Never,
    },
    TagSpec {
        name: "importClassesFrom",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ImportClassesFrom),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Indent,
    },
    TagSpec {
        name: "importMethodsFrom",
        kind: KnownTagKind::Namespace(NamespaceTagKind::ImportMethodsFrom),
        grammar: ValueGrammar::Words,
        requirement: ValueRequirement::Required,
        multiline: Multiline::Indent,
    },
    TagSpec {
        name: "eval",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::Eval),
        grammar: ValueGrammar::Code,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "evalRd",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::EvalRd),
        grammar: ValueGrammar::Code,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "evalNamespace",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::EvalNamespace),
        grammar: ValueGrammar::Code,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "template",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::Template),
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "templateVar",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::TemplateVar),
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "includeRmd",
        kind: KnownTagKind::Unsupported(UnsupportedTagKind::IncludeRmd),
        grammar: ValueGrammar::Plain,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Always,
    },
    TagSpec {
        name: "md",
        kind: KnownTagKind::MarkdownMarker,
        grammar: ValueGrammar::Marker,
        requirement: ValueRequirement::Forbidden,
        multiline: Multiline::Never,
    },
    // @noMd is diagnosed by the raw adapter; this entry prevents a duplicate
    // unknown-tag diagnostic when callers parse the semantic layer alone.
    TagSpec {
        name: "noMd",
        kind: KnownTagKind::NoMd,
        grammar: ValueGrammar::Toggle,
        requirement: ValueRequirement::Allowed,
        multiline: Multiline::Never,
    },
];

/// Finds the complete policy for one tag spelling.
#[must_use]
pub(super) fn tag_spec(name: &str) -> Option<TagSpec> {
    TAG_SPECS.iter().find(|spec| spec.name == name).copied()
}

#[cfg(test)]
mod tests {
    use super::{Multiline, ValueGrammar, ValueRequirement, tag_spec};

    #[test]
    fn every_registry_entry_has_explicit_grammar_requirement_and_policy() {
        for name in [
            "title",
            "description",
            "details",
            "return",
            "returns",
            "seealso",
            "references",
            "note",
            "format",
            "source",
            "author",
            "param",
            "name",
            "rdname",
            "aliases",
            "keywords",
            "noRd",
            "examples",
            "usage",
            "inherit",
            "inheritSection",
            "inheritParams",
            "section",
            "export",
            "exportS3Method",
            "method",
            "import",
            "importFrom",
            "rawNamespace",
            "useDynLib",
            "exportPattern",
            "exportClass",
            "exportMethod",
            "importClassesFrom",
            "importMethodsFrom",
            "eval",
            "evalRd",
            "evalNamespace",
            "template",
            "templateVar",
            "includeRmd",
            "md",
            "noMd",
        ] {
            let spec = tag_spec(name).expect("known tag must have a registry entry");
            assert!(matches!(
                spec.grammar,
                ValueGrammar::Markdown
                    | ValueGrammar::Plain
                    | ValueGrammar::Words
                    | ValueGrammar::Code
                    | ValueGrammar::RCode
                    | ValueGrammar::Param
                    | ValueGrammar::Inherit
                    | ValueGrammar::InheritParams
                    | ValueGrammar::Section
                    | ValueGrammar::Toggle
                    | ValueGrammar::Marker
            ));
            assert!(matches!(
                spec.requirement,
                ValueRequirement::Required
                    | ValueRequirement::Allowed
                    | ValueRequirement::Forbidden
            ));
            assert!(matches!(
                spec.multiline,
                Multiline::Never | Multiline::Indent | Multiline::Always
            ));
        }
    }
}
