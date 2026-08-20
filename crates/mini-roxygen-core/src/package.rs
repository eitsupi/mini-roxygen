//! Package-level inputs loaded from a package root.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use arity_parser::dcf::{self, Document, Field, ParseDiagnostic};

use crate::source::{SourceError, SourceMap};

/// The package metadata currently consumed by the documentation pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageMetadata {
    package: String,
    dependencies: BTreeSet<String>,
    encoding: Option<String>,
    collate: bool,
    lazy_data: bool,
    documentation: PackageDocumentationMetadata,
}

/// DESCRIPTION fields used as defaults for the package documentation topic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageDocumentationMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub bug_reports: Option<String>,
    pub authors_r: Option<String>,
}

impl PackageMetadata {
    /// Constructs package metadata for an in-memory package.
    pub fn new(
        package: impl Into<String>,
        encoding: Option<String>,
    ) -> Result<Self, PackageMetadataError> {
        let package = package.into();
        validate_package_name(&package)?;
        if let Some(encoding) = &encoding
            && encoding != "UTF-8"
        {
            return Err(PackageMetadataError::UnsupportedEncoding {
                encoding: encoding.clone(),
            });
        }
        Ok(Self {
            package,
            dependencies: BTreeSet::new(),
            encoding,
            collate: false,
            lazy_data: false,
            documentation: PackageDocumentationMetadata::default(),
        })
    }

    /// Returns the validated R package name.
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Returns packages named by the Depends and Imports fields.
    #[must_use]
    pub fn dependencies(&self) -> &BTreeSet<String> {
        &self.dependencies
    }

    /// Returns the declared DESCRIPTION encoding, if present.
    #[must_use]
    pub fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    /// Marks the presence of any `Collate`-family DESCRIPTION directive.
    #[must_use]
    pub fn with_collate_directive(mut self) -> Self {
        self.collate = true;
        self
    }

    /// Returns whether DESCRIPTION contains any `Collate`-family directive.
    #[must_use]
    pub fn collate(&self) -> bool {
        self.collate
    }

    /// Returns whether DESCRIPTION contains any `Collate`-family directive.
    #[must_use]
    pub fn has_collate_directive(&self) -> bool {
        self.collate
    }

    /// Returns whether DESCRIPTION declares data to be lazily loaded.
    #[must_use]
    pub fn lazy_data(&self) -> bool {
        self.lazy_data
    }

    /// Sets whether data is declared to be lazily loaded.
    #[must_use]
    pub fn with_lazy_data(mut self, lazy_data: bool) -> Self {
        self.lazy_data = lazy_data;
        self
    }

    /// Returns DESCRIPTION fields used by package-level documentation.
    #[must_use]
    pub fn documentation(&self) -> &PackageDocumentationMetadata {
        &self.documentation
    }
}

/// An error constructing metadata without reading a package from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageMetadataError {
    /// The package name is empty.
    EmptyPackage,
    /// The package name does not match R's package-name grammar.
    InvalidPackageName { package: String },
    /// The declared encoding is not supported.
    UnsupportedEncoding { encoding: String },
}

impl fmt::Display for PackageMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPackage => formatter.write_str("Package is empty"),
            Self::InvalidPackageName { package } => {
                write!(formatter, "invalid R package name {package:?}")
            }
            Self::UnsupportedEncoding { encoding } => {
                write!(formatter, "unsupported declared Encoding {encoding:?}")
            }
        }
    }
}

impl Error for PackageMetadataError {}

/// The source files and package metadata needed for one documentation run.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageInputs {
    /// R source files under the package's `R/` directory.
    pub sources: SourceMap,
    /// Metadata extracted from the package's DESCRIPTION file.
    pub metadata: PackageMetadata,
}

impl PackageInputs {
    /// Loads DESCRIPTION and R sources from a package root.
    pub fn from_package_root(root: impl AsRef<Path>) -> Result<Self, PackageInputError> {
        let root = root.as_ref();
        let description_path = root.join("DESCRIPTION");
        let bytes = fs::read(&description_path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                PackageInputError::DescriptionMissing {
                    path: description_path.clone(),
                }
            } else {
                PackageInputError::DescriptionIo {
                    path: description_path.clone(),
                    source,
                }
            }
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|source| {
            PackageInputError::DescriptionInvalidUtf8 {
                path: description_path.clone(),
                source,
            }
        })?;
        let parsed = dcf::parse(text);
        if !parsed.diagnostics.is_empty() {
            return Err(PackageInputError::DescriptionMalformed {
                path: description_path,
                diagnostics: parsed.diagnostics,
            });
        }
        let description = parsed.document();

        let package = match description_field(&description, text, "Package") {
            None => {
                return Err(PackageInputError::PackageMissing {
                    path: description_path,
                });
            }
            Some(package) if package.is_empty() => {
                return Err(PackageInputError::PackageEmpty {
                    path: description_path,
                });
            }
            Some(package) => package,
        };
        validate_package_name(&package).map_err(|error| match error {
            PackageMetadataError::InvalidPackageName { package } => {
                PackageInputError::PackageInvalid {
                    path: description_path.clone(),
                    package,
                }
            }
            PackageMetadataError::EmptyPackage => PackageInputError::PackageEmpty {
                path: description_path.clone(),
            },
            PackageMetadataError::UnsupportedEncoding { .. } => {
                unreachable!("encoding is validated separately")
            }
        })?;

        let encoding = description_field(&description, text, "Encoding");
        if let Some(encoding) = &encoding
            && encoding != "UTF-8"
        {
            return Err(PackageInputError::UnsupportedEncoding {
                path: description_path,
                encoding: encoding.clone(),
            });
        }
        let metadata = PackageMetadata {
            package,
            dependencies: description_dependencies(&description, text),
            encoding,
            collate: has_collate_directive(&description, text),
            lazy_data: description_field_raw(&description, text, "LazyData").is_some_and(|value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "yes")
            }),
            documentation: PackageDocumentationMetadata {
                title: description_field(&description, text, "Title"),
                description: description_field(&description, text, "Description"),
                url: description_field(&description, text, "URL"),
                bug_reports: description_field_raw(&description, text, "BugReports"),
                authors_r: description_field_raw(&description, text, "Authors@R"),
            },
        };
        let sources = SourceMap::from_package_root(root).map_err(|source| {
            let path = source.path().to_path_buf();
            PackageInputError::Source { path, source }
        })?;
        Ok(Self { sources, metadata })
    }
}

fn description_dependencies(document: &Document, text: &str) -> BTreeSet<String> {
    ["Depends", "Imports"]
        .into_iter()
        .filter_map(|name| description_field_node(document, text, name))
        .flat_map(|field| dcf::dependency_entries(&field))
        .map(|entry| entry.name.to_string())
        .filter(|name| *name != "R" && valid_package_name(name))
        .collect()
}

fn valid_package_name(package: &str) -> bool {
    validate_package_name(package).is_ok()
}

fn description_field(document: &Document, text: &str, name: &str) -> Option<String> {
    description_field_raw(document, text, name).map(|value| {
        // R read.dcf drops the empty first segment of `Field:\n value`.
        // arity deliberately preserves it in folded_value(), so normalize it
        // for fields that previously came from r-description's typed API.
        value.strip_prefix('\n').unwrap_or(&value).to_owned()
    })
}

fn description_field_raw(document: &Document, text: &str, name: &str) -> Option<String> {
    description_field_node(document, text, name).map(|field| field.folded_value())
}

fn description_field_node(document: &Document, text: &str, name: &str) -> Option<Field> {
    // R's read.dcf resolves duplicate fields to the last value. Do not use
    // Document::field, whose deliberate first-wins policy serves other
    // consumers. The exact-name check rejects `Package : value`: arity's
    // convenience name() trims whitespace before the colon, while read.dcf
    // treats that as the distinct field name `Package `.
    document
        .fields()
        .filter(|field| field.name() == name && exact_field_name(field, text))
        .last()
}

fn exact_field_name(field: &Field, text: &str) -> bool {
    let name_end: usize = field.name_range().end().into();
    let value_start: usize = field.value_range().start().into();
    text.get(name_end..value_start)
        .is_some_and(|between| between.starts_with(':'))
}

fn has_collate_directive(document: &Document, text: &str) -> bool {
    ["Collate", "Collate.unix", "Collate.windows"]
        .into_iter()
        .any(|field| description_field_node(document, text, field).is_some())
}

/// An error loading package inputs from disk.
#[derive(Debug)]
pub enum PackageInputError {
    /// The package has no DESCRIPTION file.
    DescriptionMissing { path: PathBuf },
    /// DESCRIPTION could not be read.
    DescriptionIo { path: PathBuf, source: io::Error },
    /// DESCRIPTION is not valid UTF-8.
    DescriptionInvalidUtf8 {
        path: PathBuf,
        source: std::str::Utf8Error,
    },
    /// DESCRIPTION is not valid DCF/Deb822.
    DescriptionMalformed {
        path: PathBuf,
        diagnostics: Vec<ParseDiagnostic>,
    },
    /// DESCRIPTION has no Package field.
    PackageMissing { path: PathBuf },
    /// DESCRIPTION's Package field is empty.
    PackageEmpty { path: PathBuf },
    /// DESCRIPTION's Package field is not a valid R package name.
    PackageInvalid { path: PathBuf, package: String },
    /// DESCRIPTION declares an encoding this implementation does not read.
    UnsupportedEncoding { path: PathBuf, encoding: String },
    /// An R source file could not be loaded.
    Source { path: PathBuf, source: SourceError },
}

impl PackageInputError {
    /// Returns the path involved in the failed package-input operation.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::DescriptionMissing { path }
            | Self::DescriptionIo { path, .. }
            | Self::DescriptionInvalidUtf8 { path, .. }
            | Self::DescriptionMalformed { path, .. }
            | Self::PackageMissing { path }
            | Self::PackageEmpty { path }
            | Self::PackageInvalid { path, .. }
            | Self::UnsupportedEncoding { path, .. }
            | Self::Source { path, .. } => path,
        }
    }
}

impl fmt::Display for PackageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DescriptionMissing { path } => {
                write!(formatter, "DESCRIPTION is missing: {}", path.display())
            }
            Self::DescriptionIo { path, source } => {
                write!(
                    formatter,
                    "failed to read DESCRIPTION {}: {source}",
                    path.display()
                )
            }
            Self::DescriptionInvalidUtf8 { path, source } => {
                write!(
                    formatter,
                    "DESCRIPTION {} is not valid UTF-8: {source}",
                    path.display()
                )
            }
            Self::DescriptionMalformed { path, diagnostics } => {
                write!(formatter, "malformed DESCRIPTION {}", path.display())?;
                if let Some(diagnostic) = diagnostics.first() {
                    write!(formatter, ": {}", diagnostic.message)?;
                }
                Ok(())
            }
            Self::PackageMissing { path } => {
                write!(
                    formatter,
                    "DESCRIPTION {} is missing Package",
                    path.display()
                )
            }
            Self::PackageEmpty { path } => {
                write!(
                    formatter,
                    "DESCRIPTION {} has an empty Package",
                    path.display()
                )
            }
            Self::PackageInvalid { path, package } => write!(
                formatter,
                "DESCRIPTION {} has invalid Package name {package:?}",
                path.display()
            ),
            Self::UnsupportedEncoding { path, encoding } => write!(
                formatter,
                "DESCRIPTION {} declares unsupported Encoding {encoding:?}",
                path.display()
            ),
            Self::Source { path, source } => {
                write!(
                    formatter,
                    "failed to load package source {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for PackageInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DescriptionIo { source, .. } => Some(source),
            Self::DescriptionInvalidUtf8 { source, .. } => Some(source),
            Self::Source { source, .. } => Some(source),
            Self::DescriptionMissing { .. }
            | Self::DescriptionMalformed { .. }
            | Self::PackageMissing { .. }
            | Self::PackageEmpty { .. }
            | Self::PackageInvalid { .. }
            | Self::UnsupportedEncoding { .. } => None,
        }
    }
}

/// Checks a name against R's package-name grammar.
///
/// R states the rule as `[[:alpha:]][[:alnum:].]*[[:alnum:]]`, which is
/// stricter than "starts with a letter, then letters, digits and dots" in two
/// ways worth spelling out: the trailing class makes a single character too
/// short, and it forbids a trailing dot. Both are rejected by R itself, so
/// accepting them here would mean documenting a package R will not install.
fn validate_package_name(package: &str) -> Result<(), PackageMetadataError> {
    let mut chars = package.chars();
    let Some(first) = chars.next() else {
        return Err(PackageMetadataError::EmptyPackage);
    };
    let Some(last) = chars.next_back() else {
        // Only the leading character: the grammar needs a trailing one too.
        return Err(PackageMetadataError::InvalidPackageName {
            package: package.to_owned(),
        });
    };
    if !first.is_ascii_alphabetic()
        || !last.is_ascii_alphanumeric()
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '.')
    {
        return Err(PackageMetadataError::InvalidPackageName {
            package: package.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{PackageInputError, PackageInputs, PackageMetadata, PackageMetadataError};

    fn write_description(root: &Path, content: &[u8]) -> PathBuf {
        let path = root.join("DESCRIPTION");
        fs::write(&path, content).expect("DESCRIPTION should be writable");
        path
    }

    fn valid_description() -> &'static [u8] {
        b"Package: example.pkg\nVersion: 0.0.1\n"
    }

    #[test]
    fn valid_description_loads_sources_and_metadata() {
        let root = tempdir().expect("temporary package root");
        write_description(root.path(), valid_description());
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        fs::write(
            root.path().join("R/example.R"),
            "example <- function() NULL\n",
        )
        .expect("source should be writable");

        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        assert_eq!(inputs.metadata.package(), "example.pkg");
        assert_eq!(inputs.metadata.encoding(), None);
        assert!(!inputs.metadata.collate());
        assert_eq!(inputs.sources.len(), 1);
    }

    #[test]
    fn depends_and_imports_are_parsed_without_including_suggests_or_r() {
        let root = tempdir().expect("temporary package root");
        write_description(
            root.path(),
            b"Package: example\nVersion: 0.0.1\nDepends: R (>= 4.0), dep.one\nImports: dep.two (>= 1.0), dep.three\nSuggests: ignored\n",
        );
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        assert_eq!(
            inputs.metadata.dependencies(),
            &std::collections::BTreeSet::from([
                "dep.one".to_owned(),
                "dep.three".to_owned(),
                "dep.two".to_owned(),
            ])
        );
    }

    #[test]
    fn collate_presence_is_recorded_without_interpreting_its_value() {
        for (field, expected) in [
            ("", false),
            ("Collate: a.R\n", true),
            ("Collate: a.R\n b.R\n", true),
            ("Collate:\n", true),
            ("Collate.unix:\n", true),
            ("Collate.windows:\n", true),
            ("Description: text\n Collate: not a field\n", false),
            ("CollateNotes: a.R\n", false),
        ] {
            let root = tempdir().expect("temporary package root");
            write_description(
                root.path(),
                format!("Package: example\nVersion: 0.0.1\n{field}").as_bytes(),
            );
            fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
            fs::write(
                root.path().join("R/example.R"),
                "example <- function() NULL\n",
            )
            .expect("source should be writable");
            let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
            assert_eq!(inputs.metadata.collate(), expected, "{field:?}");
            let model = crate::model::build_package_model_with_metadata(
                &inputs.sources,
                Vec::new(),
                &inputs.metadata,
            );
            assert_eq!(model.package.collate, expected, "{field:?}");
        }
    }

    #[test]
    fn metadata_can_be_constructed_for_in_memory_tests() {
        let metadata = PackageMetadata::new("inMemory", Some("UTF-8".to_owned()))
            .expect("metadata should be valid");
        assert_eq!(metadata.package(), "inMemory");
        assert_eq!(metadata.encoding(), Some("UTF-8"));
        assert!(!metadata.has_collate_directive());
        assert!(metadata.with_collate_directive().has_collate_directive());
    }

    #[test]
    fn folded_description_field_survives_following_fields() {
        let description = "BugReports: https://example.org/issues\n continuation\nDescription: later field\n continuation\n";
        assert_eq!(
            super::description_field_raw(
                &arity_parser::dcf::parse(description).document(),
                description,
                "BugReports"
            ),
            Some("https://example.org/issues\ncontinuation".to_owned())
        );
    }

    #[test]
    fn description_fields_match_read_dcf_duplicate_and_name_rules() {
        let root = tempdir().expect("temporary package root");
        let path = write_description(root.path(), b"Package : spaced\n");
        let error = PackageInputs::from_package_root(root.path())
            .expect_err("the exact Package field should be missing");
        assert!(matches!(error, PackageInputError::PackageMissing { .. }));
        assert_eq!(error.path(), path);

        write_description(root.path(), b"Package: first\nPackage: second\n");
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        assert_eq!(inputs.metadata.package(), "second");
    }

    #[test]
    fn description_edge_fields_keep_r_semantics() {
        let root = tempdir().expect("temporary package root");
        write_description(
            root.path(),
            "Package: example.pkg\nTitle: café\nVersion: 0.0.1\nDepends: R (>= 4.0), dep.one (>= 1.0, < 2.0)\nImports:\n dep.two\nLazyData: yes\nCollate.unix:\n a.R\nBugReports: not a URL\n".as_bytes(),
        );
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        assert_eq!(inputs.metadata.package(), "example.pkg");
        assert_eq!(
            inputs.metadata.documentation().title.as_deref(),
            Some("café")
        );
        assert_eq!(
            inputs.metadata.dependencies(),
            &std::collections::BTreeSet::from(["dep.one".to_owned(), "dep.two".to_owned()])
        );
        assert!(inputs.metadata.lazy_data());
        assert!(inputs.metadata.collate());
        assert_eq!(
            inputs.metadata.documentation().bug_reports.as_deref(),
            Some("not a URL")
        );
    }

    #[test]
    fn empty_field_value_matches_read_dcf_for_typed_fields() {
        let text = "Package: example.pkg\nDescription:\n continuation\n";
        let document = arity_parser::dcf::parse(text).document();
        assert_eq!(
            super::description_field(&document, text, "Description"),
            Some("continuation".to_owned())
        );
        assert_eq!(
            super::description_field_raw(&document, text, "Description"),
            Some("\ncontinuation".to_owned())
        );
    }

    #[test]
    fn malformed_and_orphan_lines_remain_description_errors() {
        for content in [b"Package example\n".as_slice(), b"Package: p\n\n orphan\n"] {
            let root = tempdir().expect("temporary package root");
            write_description(root.path(), content);
            let error =
                PackageInputs::from_package_root(root.path()).expect_err("load should fail");
            assert!(matches!(
                error,
                PackageInputError::DescriptionMalformed { .. }
            ));
        }
    }

    #[test]
    fn missing_description_carries_its_path() {
        let root = tempdir().expect("temporary package root");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(
            error,
            PackageInputError::DescriptionMissing { .. }
        ));
        assert_eq!(error.path(), root.path().join("DESCRIPTION"));
    }

    #[test]
    fn unreadable_description_is_an_io_error() {
        let root = tempdir().expect("temporary package root");
        let path = root.path().join("DESCRIPTION");
        fs::create_dir(&path).expect("DESCRIPTION directory should be creatable");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(error, PackageInputError::DescriptionIo { .. }));
        assert_eq!(error.path(), path);
    }

    #[test]
    fn invalid_description_utf8_carries_its_path() {
        let root = tempdir().expect("temporary package root");
        let path = write_description(root.path(), b"Package: example\n\xff");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(
            error,
            PackageInputError::DescriptionInvalidUtf8 { .. }
        ));
        assert_eq!(error.path(), path);
    }

    #[test]
    fn malformed_description_carries_parser_error_and_path() {
        let root = tempdir().expect("temporary package root");
        let path = write_description(root.path(), b"Package example\n");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(
            error,
            PackageInputError::DescriptionMalformed { .. }
        ));
        assert_eq!(error.path(), path);
    }

    #[test]
    fn missing_and_empty_package_are_distinct() {
        let root = tempdir().expect("temporary package root");
        let path = write_description(root.path(), b"Version: 0.0.1\n");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(error, PackageInputError::PackageMissing { .. }));
        assert_eq!(error.path(), path);

        let path = write_description(root.path(), b"Package: \n");
        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(error, PackageInputError::PackageEmpty { .. }));
        assert_eq!(error.path(), path);
    }

    #[test]
    fn encoding_policy_accepts_absent_and_utf8_and_rejects_other_values() {
        let root = tempdir().expect("temporary package root");
        write_description(root.path(), valid_description());
        PackageInputs::from_package_root(root.path()).expect("absent encoding should succeed");

        write_description(root.path(), b"Package: example\nEncoding: UTF-8\n");
        let inputs = PackageInputs::from_package_root(root.path()).expect("UTF-8 should succeed");
        assert_eq!(inputs.metadata.encoding(), Some("UTF-8"));

        let path = write_description(root.path(), b"Package: example\nEncoding: latin1\n");
        let error =
            PackageInputs::from_package_root(root.path()).expect_err("encoding should fail");
        assert!(matches!(
            error,
            PackageInputError::UnsupportedEncoding { .. }
        ));
        assert_eq!(error.path(), path);
    }

    #[test]
    fn source_loading_errors_are_wrapped_with_the_source_path() {
        let root = tempdir().expect("temporary package root");
        write_description(root.path(), valid_description());
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        let path = root.path().join("R/broken.R");
        fs::write(&path, [0xff]).expect("invalid source should be writable");

        let error = PackageInputs::from_package_root(root.path()).expect_err("load should fail");
        assert!(matches!(error, PackageInputError::Source { .. }));
        assert_eq!(error.path(), Path::new("R/broken.R"));
    }

    #[test]
    fn metadata_constructor_validates_package_and_encoding() {
        assert_eq!(
            PackageMetadata::new("", None).expect_err("empty package should fail"),
            PackageMetadataError::EmptyPackage
        );
        // Cases and expectations taken from R's own grammar for a package
        // name, `[[:alpha:]][[:alnum:].]*[[:alnum:]]`, checked against R.
        for rejected in [
            "bad-name", "bad_name", "1package", "x", "pkg.", "pkg..", ".",
        ] {
            assert!(
                matches!(
                    PackageMetadata::new(rejected, None),
                    Err(PackageMetadataError::InvalidPackageName { .. })
                ),
                "{rejected:?} should be rejected"
            );
        }
        for accepted in ["xy", "pkg.a", "myPkg", "a1", "p.k.g"] {
            assert!(
                PackageMetadata::new(accepted, None).is_ok(),
                "{accepted:?} should be accepted"
            );
        }
        assert!(matches!(
            PackageMetadata::new("example", Some("latin1".to_owned())),
            Err(PackageMetadataError::UnsupportedEncoding { .. })
        ));
    }
}
