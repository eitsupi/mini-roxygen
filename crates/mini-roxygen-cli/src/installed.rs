//! Explicit installed-library S3 metadata discovery for the CLI.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rd_rds::package::{InstalledMetadataError, MetadataField, NamespaceMetadata, PackageMeta};

use crate::base_catalog::{self, SupportedRMinor};
use crate::documentation::InstalledDocumentationProvider;
use crate::provider::{self, ComposedS3Provider};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataWarning {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct LoadedProviders {
    pub(crate) s3: ComposedS3Provider,
    pub(crate) documentation: InstalledDocumentationProvider,
    pub(crate) warnings: Vec<MetadataWarning>,
}

pub(crate) fn load_providers(
    library_paths: &[PathBuf],
    dependencies: &BTreeSet<String>,
) -> LoadedProviders {
    let (minor, mut warnings) = select_base_catalog(library_paths);
    let visible = visible_packages(library_paths, dependencies);
    let mut installed = BTreeSet::new();
    for package_path in visible.values() {
        let metadata = match NamespaceMetadata::read_installed(package_path) {
            Ok(metadata) => metadata,
            Err(InstalledMetadataError::Read { path, .. }) if is_missing(&path) => continue,
            Err(InstalledMetadataError::Read { path, source }) => {
                warnings.push(MetadataWarning {
                    path,
                    message: format!("cannot read installed S3 metadata: {source}"),
                });
                continue;
            }
            Err(InstalledMetadataError::View { path, source }) => {
                warnings.push(MetadataWarning {
                    path,
                    message: format!("invalid installed S3 metadata: {source}"),
                });
                continue;
            }
            Err(error) => {
                let path = error.path().to_path_buf();
                warnings.push(MetadataWarning {
                    path,
                    message: format!("cannot read installed S3 metadata: {error}"),
                });
                continue;
            }
        };
        match extract_s3_generics_from_metadata(&metadata) {
            Ok(generics) => installed.extend(generics),
            Err(message) => warnings.push(MetadataWarning {
                path: package_path.join("Meta/nsInfo.rds"),
                message,
            }),
        }
    }
    LoadedProviders {
        s3: provider::compose(installed, base_catalog::catalog_for(minor)),
        documentation: InstalledDocumentationProvider::new(library_paths),
        warnings,
    }
}

fn select_base_catalog(library_paths: &[PathBuf]) -> (SupportedRMinor, Vec<MetadataWarning>) {
    if library_paths.is_empty() {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: PathBuf::from("--r-lib-path"),
                message: fallback_message(None, "was not specified"),
            }],
        );
    }

    let Some(base_path) = find_base_package(library_paths) else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: library_paths[0].clone(),
                message: fallback_message(None, "the base package was not found"),
            }],
        );
    };
    let metadata_path = package_metadata_path(&base_path);
    let metadata = match PackageMeta::read_installed(&base_path) {
        Ok(metadata) => metadata,
        Err(InstalledMetadataError::Read { path, .. }) => {
            return (
                SupportedRMinor::R4_6,
                vec![MetadataWarning {
                    path,
                    message: fallback_message(None, "base/Meta/package.rds could not be read"),
                }],
            );
        }
        Err(InstalledMetadataError::View { path, .. }) => {
            return (
                SupportedRMinor::R4_6,
                vec![MetadataWarning {
                    path,
                    message: fallback_message(None, "base/Meta/package.rds could not be decoded"),
                }],
            );
        }
        Err(error) => {
            return (
                SupportedRMinor::R4_6,
                vec![MetadataWarning {
                    path: error.path().to_path_buf(),
                    message: fallback_message(None, "base/Meta/package.rds could not be read"),
                }],
            );
        }
    };
    let Some(built) = metadata.built() else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: metadata_path,
                message: fallback_message(None, "base/Meta/package.rds has no Built.R field"),
            }],
        );
    };
    let version = built.r_version().to_string();
    let components = built.r_version().components();
    let Some((&major, &minor)) = components.first().zip(components.get(1)) else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: metadata_path,
                message: fallback_message(Some(&version), "the Built.R field could not be parsed"),
            }],
        );
    };
    match (major, minor) {
        (4, 5) => (SupportedRMinor::R4_5, Vec::new()),
        (4, 6) => (SupportedRMinor::R4_6, Vec::new()),
        _ => (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: metadata_path,
                message: fallback_message(
                    Some(&version),
                    "the detected minor is outside the supported range",
                ),
            }],
        ),
    }
}

fn find_base_package(library_paths: &[PathBuf]) -> Option<PathBuf> {
    library_paths
        .iter()
        .map(|library| library.join("base"))
        .find(|path| path.is_dir())
}

fn fallback_message(version: Option<&str>, reason: &str) -> String {
    let detected = version
        .map(|version| format!(" detected version {version};"))
        .unwrap_or_default();
    format!("{reason};{detected} supported R 4.5--4.6; using R 4.6 semantics fallback")
}

#[cfg(test)]
fn extract_s3_generics(root: &rd_rds::RObject) -> Result<BTreeSet<String>, String> {
    let metadata = NamespaceMetadata::from_object(root)
        .map_err(|error| format!("invalid installed S3 metadata: {error}"))?;
    extract_s3_generics_from_metadata(&metadata)
}

fn extract_s3_generics_from_metadata(
    metadata: &NamespaceMetadata,
) -> Result<BTreeSet<String>, String> {
    match metadata.s3_generic_evidence() {
        MetadataField::Missing => Ok(BTreeSet::new()),
        MetadataField::Present(generics) => Ok(generics.iter().cloned().collect()),
        MetadataField::Invalid(error) => Err(format!("invalid installed S3 metadata: {error}")),
        MetadataField::UnsupportedSchema { description } => Err(format!(
            "unsupported installed S3 metadata schema: {description}"
        )),
        _ => Err("unsupported installed S3 metadata field".to_owned()),
    }
}

pub(crate) fn render_warning(warning: &MetadataWarning) -> String {
    format!("warning: {} {}", warning.path.display(), warning.message)
}

fn visible_packages(
    library_paths: &[PathBuf],
    dependencies: &BTreeSet<String>,
) -> BTreeMap<String, PathBuf> {
    let mut seen = BTreeSet::new();
    let mut visible = BTreeMap::new();
    for library in library_paths {
        let Ok(entries) = fs::read_dir(library) else {
            eprintln!("warning: cannot read R library path {}", library.display());
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !valid_package_name(name) {
                continue;
            }
            let explicit_dependency = dependencies.contains(name);
            let has_marker = package_metadata_path(&path).is_file();
            if !explicit_dependency && !has_marker {
                continue;
            }
            if !seen.insert(name.to_owned()) {
                continue;
            }
            if explicit_dependency {
                visible.insert(name.to_owned(), path);
                continue;
            }
            let priority = read_package_metadata(&path)
                .map(|metadata| {
                    metadata
                        .description_field("Priority")
                        .flatten()
                        .is_some_and(|priority| {
                            matches!(
                                priority.trim().to_ascii_lowercase().as_str(),
                                "base" | "recommended"
                            )
                        })
                })
                .unwrap_or(false);
            if priority {
                visible.insert(name.to_owned(), path);
            }
        }
    }
    visible
}

fn package_metadata_path(package_path: &Path) -> PathBuf {
    package_path.join("Meta/package.rds")
}

fn read_package_metadata(package_path: &Path) -> Option<PackageMeta> {
    PackageMeta::read_installed(package_path).ok()
}

fn valid_package_name(package: &str) -> bool {
    let mut chars = package.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(last) = chars.next_back() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && last.is_ascii_alphanumeric()
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '.')
}

fn is_missing(path: &Path) -> bool {
    fs::metadata(path)
        .err()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use rd_rds::package::{
        BodyValidation, DefaultPresence, FormalsInspection, InspectionExtent, InstalledCodeDb,
    };
    use rd_rds::{
        Attribute, Attributes, NativeEncodingSource, REncoding, RObject, RStr, RValue, Symbol,
    };
    use tempfile::tempdir;

    use super::{
        MetadataField, MetadataWarning, NamespaceMetadata, extract_s3_generics,
        package_metadata_path, render_warning, select_base_catalog, visible_packages,
    };
    use crate::base_catalog::SupportedRMinor;

    const POSITIVE_NSINFO: &[u8] = include_bytes!("../tests/fixtures/nsinfo-positive.rds");
    const PACKAGE_META_R45: &[u8] = include_bytes!("../tests/fixtures/package-meta-r45.rds");
    const PACKAGE_META_R46: &[u8] = include_bytes!("../tests/fixtures/package-meta-r46.rds");
    const PACKAGE_META_R44: &[u8] = include_bytes!("../tests/fixtures/package-meta-r44.rds");
    const PACKAGE_META_R47: &[u8] = include_bytes!("../tests/fixtures/package-meta-r47.rds");
    const PACKAGE_META_RECOMMENDED: &[u8] =
        include_bytes!("../tests/fixtures/package-meta-recommended.rds");
    const PACKAGE_META_NO_PRIORITY: &[u8] =
        include_bytes!("../tests/fixtures/package-meta-no-priority.rds");
    const PACKAGE_META_NO_BUILT: &[u8] =
        include_bytes!("../tests/fixtures/package-meta-no-built.rds");

    fn r_string(value: &str) -> RStr {
        RStr::new(
            value.as_bytes(),
            REncoding::Utf8,
            NativeEncodingSource::AssumedUtf8,
        )
    }

    fn named_list(entries: &[(&str, RObject)]) -> RObject {
        RObject::from_parts(
            RValue::List(entries.iter().map(|(_, value)| value.clone()).collect()),
            Attributes::new(vec![Attribute::new(
                Symbol::from("names"),
                RObject::from_parts(
                    RValue::Character(entries.iter().map(|(name, _)| r_string(name)).collect()),
                    Attributes::default(),
                ),
            )]),
        )
    }

    #[test]
    fn missing_s3methods_is_empty_without_a_warning() {
        let root = named_list(&[]);
        assert_eq!(
            extract_s3_generics(&root).expect("metadata"),
            BTreeSet::new()
        );
    }

    #[test]
    fn invalid_s3methods_is_returned_as_a_warning_error() {
        let malformed =
            RObject::from_parts(RValue::Character(vec![RStr::Na]), Attributes::default());
        let root = named_list(&[("S3methods", malformed)]);
        assert!(extract_s3_generics(&root).is_err());
    }

    #[test]
    fn namespace_metadata_preserves_unsupported_registration_schema() {
        let matrix = RObject::from_parts(
            RValue::Character(vec![r_string("print"), r_string("class")]),
            Attributes::new(vec![Attribute::new(
                Symbol::from("dim"),
                RObject::from_parts(
                    RValue::Integer(vec![Some(1), Some(2)]),
                    Attributes::default(),
                ),
            )]),
        );
        let root = named_list(&[("S3methods", matrix)]);
        assert_eq!(
            extract_s3_generics(&root).expect("generic evidence"),
            BTreeSet::from(["print".to_owned()])
        );
        let metadata = NamespaceMetadata::from_object(&root).expect("metadata");
        assert!(matches!(
            metadata.s3_registrations(),
            MetadataField::UnsupportedSchema { .. }
        ));
    }

    #[test]
    fn extracts_the_first_column_from_a_non_empty_nsinfo_matrix() {
        let root = rd_rds::file::from_bytes(POSITIVE_NSINFO).expect("fixture should decode");
        assert_eq!(
            extract_s3_generics(&root).expect("S3methods should decode"),
            BTreeSet::from(["+".to_owned(), "print".to_owned()])
        );
    }

    #[test]
    fn installed_code_db_inspects_stats_median_formals_without_validating_body() {
        let libraries = crate::test_support::installed_r_libraries();
        let Some(package) = libraries
            .iter()
            .map(|library| library.join("stats"))
            .find(|package| package.join("R/stats.rdx").is_file())
        else {
            assert!(
                !crate::test_support::installed_docs_required(),
                "required installed R docs are unavailable: stats code database was not found"
            );
            return;
        };

        let database = match InstalledCodeDb::open(&package) {
            Ok(database) => database,
            Err(error) if crate::test_support::installed_docs_required() => {
                panic!("required installed stats code database could not be opened: {error}")
            }
            Err(_) => return,
        };
        let inspection = match database.inspect_stored_binding("median") {
            Ok(inspection) => inspection,
            Err(error) if crate::test_support::installed_docs_required() => {
                panic!("required installed stats::median could not be inspected: {error}")
            }
            Err(_) => return,
        };

        let FormalsInspection::Available(formals) = inspection.formals() else {
            panic!(
                "stats::median did not expose closure formals: {:?}",
                inspection.formals()
            );
        };
        assert_eq!(
            formals
                .iter()
                .map(|formal| formal.name())
                .collect::<Vec<_>>(),
            ["x", "na.rm", "..."]
        );
        assert_eq!(
            formals
                .iter()
                .map(|formal| formal.default())
                .collect::<Vec<_>>(),
            [
                DefaultPresence::Absent,
                DefaultPresence::Present,
                DefaultPresence::Absent
            ]
        );
        assert!(matches!(
            inspection.extent(),
            InspectionExtent::ThroughFormals {
                body_validation: BodyValidation::NotValidated,
                ..
            }
        ));
    }

    #[test]
    fn installed_base_package_metadata_selects_the_matching_catalog() {
        let libraries = crate::test_support::installed_r_libraries();
        let Some(base) = libraries
            .iter()
            .map(|library| library.join("base"))
            .find(|package| package_metadata_path(package).is_file())
        else {
            crate::test_support::require_installed_documentation(&libraries, "base");
            return;
        };

        let metadata = match rd_rds::package::PackageMeta::read_installed(&base) {
            Ok(metadata) => metadata,
            Err(error) if crate::test_support::installed_docs_required() => {
                panic!("required installed base metadata could not be read: {error}")
            }
            Err(_) => return,
        };
        assert_eq!(metadata.description_field("Priority"), Some(Some("base")));
        let Some(built) = metadata.built() else {
            assert!(
                !crate::test_support::installed_docs_required(),
                "required installed base metadata has no Built.R field"
            );
            return;
        };
        let version = built.r_version().components();
        let expected = match version.get(0..2) {
            Some([4, 5]) => SupportedRMinor::R4_5,
            Some([4, 6]) => SupportedRMinor::R4_6,
            _ => {
                assert!(
                    !crate::test_support::installed_docs_required(),
                    "required installed base metadata reports an unsupported R version"
                );
                return;
            }
        };
        assert_eq!(select_base_catalog(&libraries).0, expected);
    }

    #[test]
    fn load_providers_reads_the_fixture_from_a_visible_dependency() {
        let library = tempdir().expect("library");
        let package = library.path().join("dep");
        fs::create_dir_all(package.join("Meta")).expect("package");
        fs::write(package.join("Meta/nsInfo.rds"), POSITIVE_NSINFO).expect("metadata");

        let loaded = super::load_providers(
            &[library.path().to_owned()],
            &BTreeSet::from(["dep".to_owned()]),
        );
        assert_eq!(loaded.warnings.len(), 1);
        assert!(
            loaded.warnings[0]
                .message
                .contains("base package was not found")
        );
        assert!(loaded.s3.generics.contains("+"));
        assert!(loaded.s3.generics.contains("print"));
        assert!(loaded.s3.generics.contains("mean"));
    }

    #[test]
    fn load_providers_preserves_installed_metadata_read_warning_context() {
        let library = tempdir().expect("library");
        let package = library.path().join("dep");
        fs::create_dir_all(package.join("Meta")).expect("package");
        fs::write(package.join("Meta/nsInfo.rds"), b"not an RDS file").expect("metadata");

        let loaded = super::load_providers(
            &[library.path().to_owned()],
            &BTreeSet::from(["dep".to_owned()]),
        );
        assert_eq!(loaded.warnings.len(), 2);
        let warning = loaded
            .warnings
            .iter()
            .find(|warning| warning.path == package.join("Meta/nsInfo.rds"))
            .expect("namespace metadata warning");
        assert!(
            warning
                .message
                .starts_with("cannot read installed S3 metadata: ")
        );
    }

    #[test]
    fn explicit_dependencies_remain_visible_when_package_metadata_is_missing_or_invalid() {
        let library = tempdir().expect("library");
        let missing = library.path().join("missing");
        fs::create_dir(&missing).expect("missing metadata package");
        let invalid = library.path().join("invalid");
        fs::create_dir_all(invalid.join("Meta")).expect("invalid metadata directory");
        fs::write(invalid.join("Meta/package.rds"), b"not an RDS file")
            .expect("invalid package metadata");
        let visible = visible_packages(
            &[library.path().to_owned()],
            &BTreeSet::from(["missing".to_owned(), "invalid".to_owned()]),
        );
        assert_eq!(visible.get("missing"), Some(&missing));
        assert_eq!(visible.get("invalid"), Some(&invalid));
    }

    fn package(root: &std::path::Path, name: &str, priority: Option<&str>) {
        let path = root.join(name);
        fs::create_dir_all(path.join("Meta")).expect("package directory");
        let metadata = match priority {
            None => PACKAGE_META_NO_PRIORITY,
            Some("base") => PACKAGE_META_R46,
            Some("recommended") => PACKAGE_META_RECOMMENDED,
            Some(other) => panic!("unsupported fixture priority {other}"),
        };
        fs::write(path.join("Meta/package.rds"), metadata).expect("package metadata");
    }

    #[test]
    fn selects_only_visible_packages_and_honors_library_order() {
        let first = tempdir().expect("first library");
        let second = tempdir().expect("second library");
        package(first.path(), "duplicate", None);
        package(second.path(), "duplicate", Some("recommended"));
        package(first.path(), "shadowed", None);
        package(second.path(), "shadowed", Some("recommended"));
        package(first.path(), "suggested", None);
        package(first.path(), "recommended", Some("recommended"));
        package(first.path(), "basepriority", Some("base"));
        package(first.path(), "dependency", None);
        package(first.path(), "current", None);
        fs::create_dir(first.path().join("unmarked")).expect("unmarked package directory");
        package(second.path(), "unmarked", Some("recommended"));
        let dependencies = BTreeSet::from(["dependency".to_owned(), "duplicate".to_owned()]);

        let visible = visible_packages(
            &[first.path().to_owned(), second.path().to_owned()],
            &dependencies,
        );
        assert!(visible.contains_key("dependency"));
        assert!(visible.contains_key("recommended"));
        assert!(visible.contains_key("basepriority"));
        assert!(visible.contains_key("duplicate"));
        assert!(!visible.contains_key("shadowed"));
        assert!(!visible.contains_key("suggested"));
        assert!(!visible.contains_key("current"));
        assert_eq!(visible["unmarked"], second.path().join("unmarked"));
        assert_eq!(visible["duplicate"], first.path().join("duplicate"));
    }

    fn base_library(metadata: &[u8]) -> tempfile::TempDir {
        let library = tempdir().expect("library");
        let base = library.path().join("base");
        fs::create_dir_all(base.join("Meta")).expect("base directory");
        fs::write(base.join("Meta/package.rds"), metadata).expect("base metadata");
        library
    }

    fn normalized_warning(warning: &MetadataWarning, root: &Path) -> String {
        let path = warning.path.strip_prefix(root).map_or_else(
            |_| warning.path.clone(),
            |relative| Path::new("/fixture").join(relative),
        );
        let path = path
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        render_warning(&MetadataWarning {
            path: path.into(),
            message: warning.message.clone(),
        })
    }

    #[test]
    fn supported_built_versions_select_their_minor_catalog() {
        for (metadata, expected) in [
            (PACKAGE_META_R45, SupportedRMinor::R4_5),
            (PACKAGE_META_R46, SupportedRMinor::R4_6),
        ] {
            let library = base_library(metadata);
            let (minor, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(minor, expected);
            assert!(warnings.is_empty());
        }
    }

    #[test]
    fn unsupported_or_missing_built_versions_warn_and_fallback() {
        for metadata in [PACKAGE_META_R44, PACKAGE_META_R47, PACKAGE_META_NO_BUILT] {
            let library = base_library(metadata);
            let (minor, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(minor, SupportedRMinor::R4_6);
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].message.contains("supported R 4.5--4.6"));
            assert!(warnings[0].message.contains("R 4.6 semantics fallback"));
        }
    }

    #[test]
    fn missing_path_and_base_are_warning_fallbacks() {
        let no_path = select_base_catalog(&[]);
        assert_eq!(no_path.0, SupportedRMinor::R4_6);
        assert_eq!(no_path.1[0].path, std::path::Path::new("--r-lib-path"));
        let missing = select_base_catalog(&[std::path::PathBuf::from("/fixture/missing")]);
        assert_eq!(missing.0, SupportedRMinor::R4_6);
        assert!(missing.1[0].message.contains("base package was not found"));
    }

    #[test]
    fn fallback_warning_contract_is_snapshot_stable() {
        let no_path = select_base_catalog(&[]);
        insta::assert_snapshot!(render_warning(&no_path.1[0]), @r###"warning: --r-lib-path was not specified; supported R 4.5--4.6; using R 4.6 semantics fallback"###);

        let missing = select_base_catalog(&[std::path::PathBuf::from("/fixture/missing")]);
        insta::assert_snapshot!(render_warning(&missing.1[0]), @r###"warning: /fixture/missing the base package was not found; supported R 4.5--4.6; using R 4.6 semantics fallback"###);

        for (metadata, expected) in [
            (
                PACKAGE_META_R44,
                "warning: /fixture/base/Meta/package.rds the detected minor is outside the supported range; detected version 4.4.9; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
            (
                PACKAGE_META_R47,
                "warning: /fixture/base/Meta/package.rds the detected minor is outside the supported range; detected version 4.7.0; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
            (
                PACKAGE_META_NO_BUILT,
                "warning: /fixture/base/Meta/package.rds base/Meta/package.rds has no Built.R field; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
        ] {
            let library = base_library(metadata);
            let (_, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(normalized_warning(&warnings[0], library.path()), expected);
        }
    }

    #[test]
    fn malformed_package_metadata_warning_is_precise() {
        let library = tempdir().expect("library");
        fs::create_dir_all(library.path().join("base/Meta")).expect("base metadata directory");
        fs::write(
            library.path().join("base/Meta/package.rds"),
            b"not an RDS file",
        )
        .expect("malformed metadata");
        let (_, warnings) = select_base_catalog(&[library.path().to_owned()]);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].path,
            library.path().join("base/Meta/package.rds")
        );
        assert!(
            warnings[0]
                .message
                .contains("base/Meta/package.rds could not be read")
        );
    }

    #[test]
    fn base_directory_and_metadata_failures_preserve_precedence() {
        let missing = tempdir().expect("library");
        fs::create_dir(missing.path().join("base")).expect("base directory");
        let second = base_library(PACKAGE_META_R46);
        let (missing_minor, missing_warnings) =
            select_base_catalog(&[missing.path().to_owned(), second.path().to_owned()]);
        assert_eq!(missing_minor, SupportedRMinor::R4_6);
        assert_eq!(
            missing_warnings[0].path,
            missing.path().join("base/Meta/package.rds")
        );
        assert!(
            missing_warnings[0]
                .message
                .contains("base/Meta/package.rds could not be read")
        );

        let unreadable = tempdir().expect("library");
        fs::create_dir_all(unreadable.path().join("base/Meta")).expect("metadata directory");
        fs::write(
            unreadable.path().join("base/Meta/package.rds"),
            b"not an RDS file",
        )
        .expect("unreadable metadata fixture");
        let (_, unreadable_warnings) = select_base_catalog(&[unreadable.path().to_owned()]);
        assert!(
            unreadable_warnings[0]
                .message
                .contains("base/Meta/package.rds could not be read")
        );
    }

    #[test]
    fn base_selection_honors_library_order() {
        let first = base_library(PACKAGE_META_R45);
        let second = base_library(PACKAGE_META_R46);
        assert_eq!(
            select_base_catalog(&[first.path().to_owned(), second.path().to_owned()]).0,
            SupportedRMinor::R4_5
        );
    }
}
