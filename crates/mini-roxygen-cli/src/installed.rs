//! Explicit installed-library S3 metadata discovery for the CLI.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rd_rds::matrix::CharacterMatrix;

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
        let metadata_path = package_path.join("Meta").join("nsInfo.rds");
        let root = match rd_rds::file::read(&metadata_path) {
            Ok(root) => root,
            Err(_error) if is_missing(&metadata_path) => continue,
            Err(error) => {
                warnings.push(MetadataWarning {
                    path: metadata_path.clone(),
                    message: format!("cannot read installed S3 metadata: {error}"),
                });
                continue;
            }
        };
        match extract_s3_generics(&root) {
            Ok(generics) => installed.extend(generics),
            Err(message) => warnings.push(MetadataWarning {
                path: metadata_path,
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
    let description_path = base_path.join("DESCRIPTION");
    let Some(description) = read_description(&base_path) else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: description_path,
                message: fallback_message(None, "base/DESCRIPTION could not be read"),
            }],
        );
    };
    let Some(version) = raw_field(&description, "Version") else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: description_path,
                message: fallback_message(None, "base/DESCRIPTION has no Version field"),
            }],
        );
    };
    let Some((major, minor)) = parse_major_minor(&version) else {
        return (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: description_path,
                message: fallback_message(
                    Some(version.trim()),
                    "the Version field could not be parsed",
                ),
            }],
        );
    };
    match (major, minor) {
        (4, 5) => (SupportedRMinor::R4_5, Vec::new()),
        (4, 6) => (SupportedRMinor::R4_6, Vec::new()),
        _ => (
            SupportedRMinor::R4_6,
            vec![MetadataWarning {
                path: description_path,
                message: fallback_message(
                    Some(version.trim()),
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
        .find(|path| path.is_dir() && path.join("DESCRIPTION").exists())
}

/// Selects only the first two numeric components. R patch/build components are
/// intentionally ignored for catalog selection, but every remaining component
/// must still be a non-empty integer so malformed versions cannot be accepted.
fn parse_major_minor(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    for component in parts {
        if component.is_empty() || component.parse::<u64>().is_err() {
            return None;
        }
    }
    Some((major, minor))
}

fn fallback_message(version: Option<&str>, reason: &str) -> String {
    let detected = version
        .map(|version| format!(" detected version {version};"))
        .unwrap_or_default();
    format!("{reason};{detected} supported R 4.5--4.6; using R 4.6 semantics fallback")
}

fn extract_s3_generics(root: &rd_rds::RObject) -> Result<BTreeSet<String>, String> {
    let Some(s3methods) = root.get_named("S3methods") else {
        return Err("installed metadata has no S3methods field".to_owned());
    };
    extract_s3_matrix(s3methods)
}

fn extract_s3_matrix(matrix: &rd_rds::RObject) -> Result<BTreeSet<String>, String> {
    let matrix = CharacterMatrix::try_from(matrix)
        .map_err(|error| format!("invalid S3methods matrix: {error}"))?;
    if matrix.ncol() == 0 {
        return Err("invalid S3methods matrix: no columns".to_owned());
    }
    Ok((0..matrix.nrow())
        .filter_map(|row| matrix.get(row, 0).flatten())
        .map(str::to_owned)
        .collect())
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
            if !valid_package_name(name) || !seen.insert(name.to_owned()) {
                continue;
            }
            let priority = read_description(&path)
                .as_deref()
                .and_then(|text| raw_field(text, "Priority"))
                .is_some_and(|priority| {
                    matches!(
                        priority.trim().to_ascii_lowercase().as_str(),
                        "base" | "recommended"
                    )
                });
            if dependencies.contains(name) || priority {
                visible.insert(name.to_owned(), path);
            }
        }
    }
    visible
}

fn read_description(package_path: &Path) -> Option<String> {
    String::from_utf8(fs::read(package_path.join("DESCRIPTION")).ok()?).ok()
}

fn raw_field(text: &str, field: &str) -> Option<String> {
    let mut value: Option<String> = None;
    let mut collecting = false;
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if collecting && let Some(current) = value.as_mut() {
                current.push('\n');
                current.push_str(line.trim_start());
            }
        } else if let Some((name, rest)) = line.split_once(':') {
            collecting = name == field;
            if collecting {
                value = Some(rest.trim_start().to_owned());
            }
        }
    }
    value
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

    use rd_rds::{Attribute, Attributes, RObject, RStr, RValue, Symbol};
    use tempfile::tempdir;

    use super::{
        MetadataWarning, extract_s3_generics, extract_s3_matrix, parse_major_minor, render_warning,
        select_base_catalog, visible_packages,
    };
    use crate::base_catalog::SupportedRMinor;

    const POSITIVE_NSINFO: &[u8] = include_bytes!("../tests/fixtures/nsinfo-positive.rds");

    fn matrix(rows: i32, columns: i32) -> RObject {
        RObject::from_parts(
            RValue::Character(vec![RStr::Na; (rows * columns) as usize]),
            Attributes::new(vec![Attribute::new(
                Symbol::from("dim"),
                RObject::from_parts(
                    RValue::Integer(vec![Some(rows), Some(columns)]),
                    Attributes::default(),
                ),
            )]),
        )
    }

    #[test]
    fn valid_zero_row_matrix_is_empty() {
        assert_eq!(
            extract_s3_matrix(&matrix(0, 1)).expect("matrix"),
            BTreeSet::new()
        );
    }

    #[test]
    fn missing_field_is_rejected_without_guessing() {
        let root = RObject::from_parts(RValue::List(Vec::new()), Attributes::default());
        assert!(extract_s3_generics(&root).is_err());
    }

    #[test]
    fn zero_column_and_malformed_matrices_are_rejected() {
        assert!(extract_s3_matrix(&matrix(0, 0)).is_err());
        let malformed = RObject::from_parts(RValue::Character(Vec::new()), Attributes::default());
        assert!(extract_s3_matrix(&malformed).is_err());
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
    fn load_providers_reads_the_fixture_from_a_visible_dependency() {
        let library = tempdir().expect("library");
        let package = library.path().join("dep");
        fs::create_dir_all(package.join("Meta")).expect("package");
        fs::write(package.join("DESCRIPTION"), "Package: dep\nVersion: 1.0\n")
            .expect("description");
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

    fn package(root: &std::path::Path, name: &str, priority: Option<&str>) {
        let path = root.join(name);
        fs::create_dir_all(&path).expect("package directory");
        let priority = priority
            .map(|value| format!("Priority: {value}\n"))
            .unwrap_or_default();
        fs::write(
            path.join("DESCRIPTION"),
            format!("Package: {name}\nVersion: 1.0\n{priority}"),
        )
        .expect("description");
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
        package(first.path(), "dependency", None);
        package(first.path(), "current", None);
        let dependencies = BTreeSet::from(["dependency".to_owned(), "duplicate".to_owned()]);

        let visible = visible_packages(
            &[first.path().to_owned(), second.path().to_owned()],
            &dependencies,
        );
        assert!(visible.contains_key("dependency"));
        assert!(visible.contains_key("recommended"));
        assert!(visible.contains_key("duplicate"));
        assert!(!visible.contains_key("shadowed"));
        assert!(!visible.contains_key("suggested"));
        assert!(!visible.contains_key("current"));
        assert_eq!(visible["duplicate"], first.path().join("duplicate"));
    }

    fn base_library(version: &str) -> tempfile::TempDir {
        let library = tempdir().expect("library");
        let base = library.path().join("base");
        fs::create_dir_all(&base).expect("base directory");
        fs::write(
            base.join("DESCRIPTION"),
            format!("Package: base\nVersion: {version}\nPriority: base\n"),
        )
        .expect("base description");
        library
    }

    fn normalized_warning(warning: &MetadataWarning, root: &Path) -> String {
        let path = warning.path.strip_prefix(root).map_or_else(
            |_| warning.path.clone(),
            |relative| Path::new("/fixture").join(relative),
        );
        render_warning(&MetadataWarning {
            path,
            message: warning.message.clone(),
        })
    }

    #[test]
    fn supported_patch_versions_select_their_minor_catalog() {
        for version in ["4.5.0", "4.5.3", "4.5.99"] {
            let library = base_library(version);
            assert_eq!(
                select_base_catalog(&[library.path().to_owned()]).0,
                SupportedRMinor::R4_5
            );
            assert!(
                select_base_catalog(&[library.path().to_owned()])
                    .1
                    .is_empty()
            );
        }
        for version in ["4.6.0", "4.6.1", "4.6.99"] {
            let library = base_library(version);
            assert_eq!(
                select_base_catalog(&[library.path().to_owned()]).0,
                SupportedRMinor::R4_6
            );
            assert!(
                select_base_catalog(&[library.path().to_owned()])
                    .1
                    .is_empty()
            );
        }
    }

    #[test]
    fn extra_numeric_components_are_ignored_after_the_minor() {
        let library = base_library("4.5.0.9000");
        assert_eq!(
            select_base_catalog(&[library.path().to_owned()]).0,
            SupportedRMinor::R4_5
        );
        assert!(
            select_base_catalog(&[library.path().to_owned()])
                .1
                .is_empty()
        );
    }

    #[test]
    fn version_components_after_minor_are_validated() {
        for (version, expected) in [
            ("4.5", Some((4, 5))),
            ("4.5.0", Some((4, 5))),
            ("4.5.0.9000", Some((4, 5))),
            ("4.6.99", Some((4, 6))),
            ("4.5.", None),
            ("4.5.invalid", None),
            ("4.5..1", None),
            ("4.5.0.invalid", None),
        ] {
            assert_eq!(parse_major_minor(version), expected, "{version}");
        }
    }

    #[test]
    fn unknown_old_new_and_unparseable_versions_warn_and_fallback() {
        for version in ["4.4.9", "4.7.0", "not-a-version"] {
            let library = base_library(version);
            let (minor, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(minor, SupportedRMinor::R4_6);
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].message.contains("supported R 4.5--4.6"));
            assert!(warnings[0].message.contains("R 4.6 semantics fallback"));
            if version != "not-a-version" {
                assert!(warnings[0].message.contains(version));
            }
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

        for (version, expected) in [
            (
                "4.4.9",
                "warning: /fixture/base/DESCRIPTION the detected minor is outside the supported range; detected version 4.4.9; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
            (
                "4.7.0",
                "warning: /fixture/base/DESCRIPTION the detected minor is outside the supported range; detected version 4.7.0; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
            (
                "not-a-version",
                "warning: /fixture/base/DESCRIPTION the Version field could not be parsed; detected version not-a-version; supported R 4.5--4.6; using R 4.6 semantics fallback",
            ),
        ] {
            let library = base_library(version);
            let (_, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(normalized_warning(&warnings[0], library.path()), expected);
        }
    }

    #[test]
    fn malformed_version_warning_snapshots_are_exact() {
        let trailing_dot = base_library("4.5.");
        let (_, warnings) = select_base_catalog(&[trailing_dot.path().to_owned()]);
        insta::assert_snapshot!(
            normalized_warning(&warnings[0], trailing_dot.path()),
            @r###"warning: /fixture/base/DESCRIPTION the Version field could not be parsed; detected version 4.5.; supported R 4.5--4.6; using R 4.6 semantics fallback"###
        );

        let invalid = base_library("4.5.invalid");
        let (_, warnings) = select_base_catalog(&[invalid.path().to_owned()]);
        insta::assert_snapshot!(
            normalized_warning(&warnings[0], invalid.path()),
            @r###"warning: /fixture/base/DESCRIPTION the Version field could not be parsed; detected version 4.5.invalid; supported R 4.5--4.6; using R 4.6 semantics fallback"###
        );

        let empty_component = base_library("4.5..1");
        let (_, warnings) = select_base_catalog(&[empty_component.path().to_owned()]);
        insta::assert_snapshot!(
            normalized_warning(&warnings[0], empty_component.path()),
            @r###"warning: /fixture/base/DESCRIPTION the Version field could not be parsed; detected version 4.5..1; supported R 4.5--4.6; using R 4.6 semantics fallback"###
        );

        let invalid_patch = base_library("4.5.0.invalid");
        let (_, warnings) = select_base_catalog(&[invalid_patch.path().to_owned()]);
        insta::assert_snapshot!(
            normalized_warning(&warnings[0], invalid_patch.path()),
            @r###"warning: /fixture/base/DESCRIPTION the Version field could not be parsed; detected version 4.5.0.invalid; supported R 4.5--4.6; using R 4.6 semantics fallback"###
        );
    }

    #[test]
    fn empty_and_whitespace_versions_fallback_without_panicking() {
        for version in ["", "   "] {
            let library = base_library(version);
            let (_, warnings) = select_base_catalog(&[library.path().to_owned()]);
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].message.contains("could not be parsed"));
        }
    }

    #[test]
    fn base_directory_without_or_unreadable_description_falls_back() {
        let missing = tempdir().expect("library");
        fs::create_dir(missing.path().join("base")).expect("base directory");
        let (_, missing_warnings) = select_base_catalog(&[missing.path().to_owned()]);
        assert!(
            missing_warnings[0]
                .message
                .contains("base package was not found")
        );

        let unreadable = tempdir().expect("library");
        fs::create_dir_all(unreadable.path().join("base/DESCRIPTION"))
            .expect("unreadable description fixture");
        let (_, unreadable_warnings) = select_base_catalog(&[unreadable.path().to_owned()]);
        assert!(
            unreadable_warnings[0]
                .message
                .contains("base/DESCRIPTION could not be read")
        );
    }

    #[test]
    fn base_selection_honors_library_order() {
        let first = base_library("4.5.0");
        let second = base_library("4.6.99");
        assert_eq!(
            select_base_catalog(&[first.path().to_owned(), second.path().to_owned()]).0,
            SupportedRMinor::R4_5
        );
    }
}
