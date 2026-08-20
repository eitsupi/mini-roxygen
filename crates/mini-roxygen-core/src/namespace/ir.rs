use crate::diagnostic::Diagnostics;

use super::render::quote_name;

/// The package name in an `import` or `importFrom` directive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamespacePackageName(pub(super) String);

impl NamespacePackageName {
    pub(super) fn new(value: String) -> Option<Self> {
        (!value.is_empty() && !value.contains('\0')).then_some(Self(value))
    }

    /// Returns the decoded package name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque text for an `import()` escape-hatch directive containing a comma.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamespaceVerbatim(pub(super) String);

impl NamespaceVerbatim {
    pub(super) fn new(value: String) -> Option<Self> {
        (!value.is_empty() && !value.contains('\0')).then_some(Self(value))
    }

    /// Returns the directive text without quoting or decoding it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An object name in an `export` or `importFrom` directive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamespaceObjectName(pub(super) String);

impl NamespaceObjectName {
    pub(super) fn new(value: String) -> Option<Self> {
        (!value.is_empty() && !value.contains('\0')).then_some(Self(value))
    }

    /// Returns the decoded object name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A non-empty set of object names used by `importFrom`, ordered by quoted
/// spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyNamespaceNames(pub(super) Vec<NamespaceObjectName>);

impl NonEmptyNamespaceNames {
    pub(super) fn new(names: Vec<NamespaceObjectName>) -> Option<Self> {
        (!names.is_empty()).then_some(Self(names))
    }

    /// Returns the names in deterministic quoted-spelling order.
    #[must_use]
    pub fn as_slice(&self) -> &[NamespaceObjectName] {
        &self.0
    }
}

/// An argument of an S3 method directive, retaining its quoting route.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NamespaceS3MethodArgument {
    /// A name that must be rendered through the normal automatic quoting rule.
    AutoQuoted(String),
    /// An expression supplied literally by an explicit namespace tag.
    Literal(String),
}

impl NamespaceS3MethodArgument {
    pub(super) fn literal(value: String) -> Option<Self> {
        (!value.is_empty() && !value.contains('\0')).then_some(Self::Literal(value))
    }

    fn rendered(&self) -> String {
        match self {
            Self::AutoQuoted(value) => quote_name(value),
            Self::Literal(value) => value.clone(),
        }
    }
}

/// A validated and normalized MVP namespace directive.
///
/// The `ImportFrom` variant cannot contain an empty name list because it uses
/// [`NonEmptyNamespaceNames`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceDirective {
    /// Export one object.
    Export {
        /// The object to export.
        name: NamespaceObjectName,
    },
    /// Import one whole package.
    Import {
        /// The package to import.
        package: NamespacePackageName,
    },
    /// Import an opaque comma-bearing expression verbatim.
    ImportVerbatim {
        /// The text inside `import(...)`.
        value: NamespaceVerbatim,
    },
    /// Import a package's native library, optionally selecting routines.
    UseDynLib {
        /// The rendered arguments inside `useDynLib(...)`.
        value: NamespaceVerbatim,
    },
    /// Preserve a raw NAMESPACE directive without reformatting it.
    RawNamespace {
        /// The source text of the directive.
        value: NamespaceVerbatim,
    },
    /// Export objects matching a pattern.
    ExportPattern {
        /// The pattern argument.
        pattern: NamespaceObjectName,
    },
    /// Export one S4 class.
    ExportClass {
        /// The class name.
        name: NamespaceObjectName,
    },
    /// Export one S4 method.
    ExportMethod {
        /// The method name.
        name: NamespaceObjectName,
    },
    /// Import one S4 class from a package.
    ImportClassesFrom {
        /// The source package.
        package: NamespacePackageName,
        /// The imported class.
        name: NamespaceObjectName,
    },
    /// Import one S4 method from a package.
    ImportMethodsFrom {
        /// The source package.
        package: NamespacePackageName,
        /// The imported method.
        name: NamespaceObjectName,
    },
    /// Import one or more named objects from a package.
    ImportFrom {
        /// The source package.
        package: NamespacePackageName,
        /// The non-empty imported object set.
        names: NonEmptyNamespaceNames,
    },
    /// Register one S3 method.
    S3Method {
        /// The generic function expression.
        generic: NamespaceS3MethodArgument,
        /// The S3 class expression.
        class: NamespaceS3MethodArgument,
    },
}

impl NamespaceDirective {
    pub(super) fn key(&self) -> NamespaceDirectiveKey {
        match self {
            Self::Export { name } => NamespaceDirectiveKey::Export(name.0.clone()),
            Self::Import { package } => NamespaceDirectiveKey::Import(package.0.clone()),
            Self::ImportVerbatim { value } => {
                NamespaceDirectiveKey::ImportVerbatim(value.0.clone())
            }
            Self::UseDynLib { value } => NamespaceDirectiveKey::UseDynLib(value.0.clone()),
            Self::RawNamespace { value } => NamespaceDirectiveKey::RawNamespace(value.0.clone()),
            Self::ExportPattern { pattern } => {
                NamespaceDirectiveKey::ExportPattern(pattern.0.clone())
            }
            Self::ExportClass { name } => NamespaceDirectiveKey::ExportClass(name.0.clone()),
            Self::ExportMethod { name } => NamespaceDirectiveKey::ExportMethod(name.0.clone()),
            Self::ImportClassesFrom { package, name } => NamespaceDirectiveKey::ImportClassesFrom {
                package: package.0.clone(),
                name: name.0.clone(),
            },
            Self::ImportMethodsFrom { package, name } => NamespaceDirectiveKey::ImportMethodsFrom {
                package: package.0.clone(),
                name: name.0.clone(),
            },
            Self::ImportFrom { package, names } => NamespaceDirectiveKey::ImportFrom {
                package: package.0.clone(),
                names: names.0.iter().map(|name| name.0.clone()).collect(),
            },
            Self::S3Method { .. } => NamespaceDirectiveKey::S3Method(self.render()),
        }
    }

    pub(super) fn render(&self) -> String {
        match self {
            Self::Export { name } => format!("export({})", quote_name(name.as_str())),
            Self::Import { package } => format!("import({})", quote_name(package.as_str())),
            Self::ImportVerbatim { value } => format!("import({})", value.as_str()),
            Self::UseDynLib { value } => format!("useDynLib({})", value.as_str()),
            Self::RawNamespace { value } => value.as_str().to_owned(),
            Self::ExportPattern { pattern } => {
                format!("exportPattern({})", quote_name(pattern.as_str()))
            }
            Self::ExportClass { name } => format!("exportClasses({})", quote_name(name.as_str())),
            Self::ExportMethod { name } => format!("exportMethods({})", quote_name(name.as_str())),
            Self::ImportClassesFrom { package, name } => format!(
                "importClassesFrom({},{})",
                quote_name(package.as_str()),
                quote_name(name.as_str())
            ),
            Self::ImportMethodsFrom { package, name } => format!(
                "importMethodsFrom({},{})",
                quote_name(package.as_str()),
                quote_name(name.as_str())
            ),
            Self::ImportFrom { package, names } => {
                let rendered_names = names
                    .as_slice()
                    .iter()
                    .map(|name| quote_name(name.as_str()))
                    .collect::<Vec<_>>();
                let package = quote_name(package.as_str());
                if rendered_names.len() == 1 {
                    format!("importFrom({package},{})", rendered_names[0])
                } else {
                    format!(
                        "importFrom({package},\n  {}\n)",
                        rendered_names.join(",\n  ")
                    )
                }
            }
            Self::S3Method { generic, class } => {
                format!("S3method({},{})", generic.rendered(), class.rendered())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NamespaceDirectiveKey {
    Export(String),
    Import(String),
    // This ordering is used only for semantic maps, not for rendered output.
    ImportVerbatim(String),
    UseDynLib(String),
    RawNamespace(String),
    ExportPattern(String),
    ExportClass(String),
    ExportMethod(String),
    ImportClassesFrom { package: String, name: String },
    ImportMethodsFrom { package: String, name: String },
    ImportFrom { package: String, names: Vec<String> },
    // Keyed by the whole rendered directive, not by the quoting route and not
    // by the arguments separately. The two routes can produce the same text,
    // and a literal argument may itself contain a comma, so only the complete
    // line distinguishes one registration from another.
    S3Method(String),
}

/// The result of building NAMESPACE text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceBuildOutput {
    /// Complete generated NAMESPACE contents.
    pub content: String,
    /// Validation, unsupported-directive, and rendering diagnostics.
    pub diagnostics: Diagnostics,
}
