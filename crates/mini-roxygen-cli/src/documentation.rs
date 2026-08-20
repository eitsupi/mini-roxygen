//! Installed-package documentation provider for explicit R library paths.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use mini_roxygen_core::{
    DocumentationError, DocumentationErrorKind, DocumentationProvider, InheritableTopic,
    TopicExistence, TopicRequest, project_external_topic,
};
use rd_helpdb::PackageHelpDb;

/// Reads external topics from installed packages in explicit library order.
///
/// Successful package databases and projected canonical topics are cached for
/// the lifetime of this provider. Failures are intentionally not cached.
#[derive(Debug)]
pub(crate) struct InstalledDocumentationProvider {
    library_paths: Vec<PathBuf>,
    databases: RefCell<BTreeMap<PathBuf, Rc<PackageHelpDb>>>,
    topics: RefCell<BTreeMap<(PathBuf, String), InheritableTopic>>,
}

impl InstalledDocumentationProvider {
    pub(crate) fn new(library_paths: &[PathBuf]) -> Self {
        Self {
            library_paths: library_paths.to_vec(),
            databases: RefCell::new(BTreeMap::new()),
            topics: RefCell::new(BTreeMap::new()),
        }
    }
}

impl DocumentationProvider for InstalledDocumentationProvider {
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        let TopicRequest::External { package, topic } = request else {
            return Ok(None);
        };

        if !is_safe_package_component(package) {
            return Err(documentation_error(
                DocumentationErrorKind::InvalidPackageName,
                package,
                &topic.0,
                "package name must be one normal path component",
            ));
        }

        let package_dir =
            find_installed_package(&self.library_paths, package).ok_or_else(|| {
                documentation_error(
                    DocumentationErrorKind::PackageUnavailable,
                    package,
                    &topic.0,
                    "package was not found in the configured library paths",
                )
            })?;
        let package_dir = canonical_package_path(&package_dir);

        let database = if let Some(database) = self.databases.borrow().get(&package_dir) {
            Rc::clone(database)
        } else {
            let database = PackageHelpDb::open(&package_dir).map_err(|error| {
                documentation_error(
                    DocumentationErrorKind::HelpDatabaseUnreadable,
                    package,
                    &topic.0,
                    format!("cannot open package help database: {error}"),
                )
            })?;
            let database = Rc::new(database);
            self.databases
                .borrow_mut()
                .insert(package_dir.clone(), Rc::clone(&database));
            database
        };

        let canonical_topic = database
            .resolve_alias(&topic.0)
            .map_err(|error| {
                documentation_error(
                    DocumentationErrorKind::AliasIndexUnreadable,
                    package,
                    &topic.0,
                    format!("cannot resolve help alias: {error}"),
                )
            })?
            .unwrap_or(&topic.0)
            .to_owned();
        let cache_key = (package_dir.clone(), canonical_topic.clone());
        if let Some(cached) = self.topics.borrow().get(&cache_key) {
            return Ok(Some(cached.clone()));
        }

        let raw = match database.raw_topic(&canonical_topic) {
            Ok(raw) => raw,
            Err(rd_helpdb::Error::UnknownTopic { .. }) => return Ok(None),
            Err(error) => {
                return Err(documentation_error(
                    DocumentationErrorKind::TopicUnreadable,
                    package,
                    &canonical_topic,
                    format!("cannot read help topic: {error}"),
                ));
            }
        };
        let document = rd_ast::lower_r_object(&raw).map_err(|error| {
            documentation_error(
                DocumentationErrorKind::RdLoweringFailed,
                package,
                &canonical_topic,
                format!("cannot lower help topic to Rd: {error}"),
            )
        })?;
        let projected = project_external_topic(package, &canonical_topic, &document, self);
        self.topics
            .borrow_mut()
            .insert(cache_key, projected.clone());
        Ok(Some(projected))
    }

    fn topic_exists(&self, package: &str, alias: &str) -> TopicExistence {
        if !is_safe_package_component(package) {
            return TopicExistence::Unavailable;
        }
        let Some(package_dir) = find_installed_package(&self.library_paths, package) else {
            return TopicExistence::Unavailable;
        };
        let package_dir = canonical_package_path(&package_dir);

        let database = if let Some(database) = self.databases.borrow().get(&package_dir) {
            Rc::clone(database)
        } else {
            let Ok(database) = PackageHelpDb::open(&package_dir) else {
                return TopicExistence::Unavailable;
            };
            let database = Rc::new(database);
            self.databases
                .borrow_mut()
                .insert(package_dir, Rc::clone(&database));
            database
        };

        match database.resolve_alias(alias) {
            Ok(Some(_)) => TopicExistence::Known(true),
            Ok(None) => TopicExistence::Known(false),
            Err(_) => TopicExistence::Unavailable,
        }
    }
}

fn documentation_error(
    kind: DocumentationErrorKind,
    package: &str,
    topic: &str,
    detail: impl Into<String>,
) -> DocumentationError {
    DocumentationError {
        kind,
        package: Some(package.to_owned()),
        topic: Some(topic.to_owned()),
        detail: detail.into(),
    }
}

fn canonical_package_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

fn find_installed_package(library_paths: &[PathBuf], package: &str) -> Option<PathBuf> {
    if !is_safe_package_component(package) {
        return None;
    }
    library_paths
        .iter()
        .map(|library| library.join(package))
        .find(|package_dir| is_installed_package(package_dir))
}

fn is_installed_package(package_dir: &Path) -> bool {
    package_dir
        .join("Meta")
        .join("package.rds")
        .metadata()
        .is_ok_and(|metadata| metadata.is_file())
}

fn is_safe_package_component(package: &str) -> bool {
    if package.is_empty()
        || package == "."
        || package == ".."
        || package.contains('\0')
        || package.contains('/')
        || package.contains('\\')
        || package.as_bytes().get(1) == Some(&b':')
        || Path::new(package).is_absolute()
    {
        return false;
    }

    let mut components = Path::new(package).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use mini_roxygen_core::{
        DocumentOptions, DocumentationErrorKind, DocumentationIdentity, DocumentationProvider,
        EmptyS3GenericProvider, ExternalInheritancePolicy, ExternalPolicySource,
        InheritableContent, InheritanceOptions, PackageInputs, PackageMetadata, SourceFile,
        SourceMap, TopicExistence, TopicKey, TopicRef, TopicRequest,
        document_package_with_options_and_providers,
    };
    use tempfile::tempdir;

    use super::{
        InstalledDocumentationProvider, find_installed_package, is_safe_package_component,
    };

    fn install_marker(root: &std::path::Path, package: &str) {
        fs::create_dir_all(root.join(package).join("Meta")).unwrap();
        fs::write(root.join(package).join("Meta/package.rds"), b"marker").unwrap();
    }

    /// Discovers libraries only for smoke tests. Production lookup stays
    /// explicit: callers must provide library paths to the CLI.
    fn installed_r_libraries() -> Vec<PathBuf> {
        let candidates = if let Some(paths) = env::var_os("MINI_ROXYGEN_TEST_R_LIB_PATHS") {
            env::split_paths(&paths).collect::<Vec<_>>()
        } else if let Some(home) = env::var_os("R_HOME") {
            vec![PathBuf::from(home).join("library")]
        } else {
            Command::new("R")
                .arg("RHOME")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| parse_r_home_output(&String::from_utf8_lossy(&output.stdout)))
                .map(|home| vec![home.join("library")])
                .unwrap_or_default()
        };

        let mut libraries = Vec::new();
        for path in candidates {
            if path.is_dir() && !libraries.contains(&path) {
                libraries.push(path);
            }
        }
        libraries
    }

    fn parse_r_home_output(output: &str) -> Option<PathBuf> {
        output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("WARNING:"))
            .map(PathBuf::from)
    }

    #[test]
    fn parses_r_home_output_around_warnings() {
        assert_eq!(
            parse_r_home_output("WARNING: startup notice\nfirst-r-home\n"),
            Some(PathBuf::from("first-r-home"))
        );
        assert_eq!(
            parse_r_home_output("first-r-home\nWARNING: trailing notice\nsecond-r-home\n"),
            Some(PathBuf::from("second-r-home"))
        );
        assert_eq!(parse_r_home_output("WARNING: no usable path\n"), None);
    }

    #[test]
    fn locator_prefers_the_first_library_when_both_markers_are_regular_files() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        install_marker(&first, "pkg");
        install_marker(&second, "pkg");

        assert_eq!(
            find_installed_package(&[first.clone(), second], "pkg"),
            Some(first.join("pkg"))
        );
    }

    #[test]
    fn locator_requires_a_regular_marker_and_continues_to_later_libraries() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(first.join("pkg/Meta/package.rds")).unwrap();
        install_marker(&second, "pkg");

        assert_eq!(
            find_installed_package(&[first, second.clone()], "pkg"),
            Some(second.join("pkg"))
        );
        assert_eq!(
            find_installed_package(&[temp.path().join("first")], "pkg"),
            None
        );
    }

    #[test]
    fn locator_rejects_unsafe_components_cross_platform() {
        for package in [
            "",
            ".",
            "..",
            "pkg\0name",
            "../pkg",
            "pkg/nested",
            r"pkg\nested",
            "C:pkg",
            "/pkg",
        ] {
            assert!(!is_safe_package_component(package), "{package:?}");
        }
        assert!(is_safe_package_component("pkg.with.dots"));
    }

    #[test]
    fn local_requests_are_clean_misses() {
        let provider = InstalledDocumentationProvider::new(&[]);
        let result = provider.get_topic(&TopicRequest::Local {
            topic: TopicRef("topic".into()),
        });
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn missing_package_is_classified_without_opening_arbitrary_directories() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join("pkg")).unwrap();
        let provider = InstalledDocumentationProvider::new(&[temp.path().to_owned()]);
        let error = provider
            .get_topic(&TopicRequest::External {
                package: "pkg".into(),
                topic: TopicRef("topic".into()),
            })
            .unwrap_err();
        assert_eq!(error.kind, DocumentationErrorKind::PackageUnavailable);
        assert_eq!(error.package.as_deref(), Some("pkg"));
    }

    #[test]
    fn marker_only_package_reports_an_unreadable_help_database() {
        let temp = tempdir().unwrap();
        install_marker(temp.path(), "pkg");
        let provider = InstalledDocumentationProvider::new(&[temp.path().to_owned()]);
        let error = provider
            .get_topic(&TopicRequest::External {
                package: "pkg".into(),
                topic: TopicRef("topic".into()),
            })
            .unwrap_err();
        assert_eq!(error.kind, DocumentationErrorKind::HelpDatabaseUnreadable);
    }

    #[test]
    fn invalid_package_is_classified_before_lookup() {
        let provider = InstalledDocumentationProvider::new(&[]);
        let error = provider
            .get_topic(&TopicRequest::External {
                package: "../pkg".into(),
                topic: TopicRef("topic".into()),
            })
            .unwrap_err();
        assert_eq!(error.kind, DocumentationErrorKind::InvalidPackageName);
    }

    #[test]
    fn identity_type_remains_external_for_provider_contract() {
        let identity = DocumentationIdentity::External {
            package: "pkg".into(),
            topic: "topic".into(),
        };
        assert!(matches!(identity, DocumentationIdentity::External { .. }));
    }

    #[test]
    fn missing_or_unreadable_help_indexes_make_topic_existence_unavailable() {
        let missing = InstalledDocumentationProvider::new(&[]);
        assert_eq!(
            missing.topic_exists("missing", "alias"),
            TopicExistence::Unavailable
        );

        let temp = tempdir().unwrap();
        install_marker(temp.path(), "pkg");
        let unreadable = InstalledDocumentationProvider::new(&[temp.path().to_owned()]);
        assert_eq!(
            unreadable.topic_exists("pkg", "alias"),
            TopicExistence::Unavailable
        );
    }

    #[test]
    fn installed_help_alias_and_canonical_requests_share_topic_cache() {
        let libraries = installed_r_libraries();
        if libraries.is_empty() {
            println!("skipping: no standard installed R library is available");
            return;
        }

        for library in libraries {
            let package_dir = library.join("utils");
            if !package_dir.join("Meta/package.rds").is_file() {
                println!(
                    "skipping: installed utils package is unavailable in {}",
                    library.display()
                );
                continue;
            }

            let database = rd_helpdb::PackageHelpDb::open(&package_dir).unwrap();
            let Some((alias, canonical)) = database.aliases().unwrap().into_iter().next() else {
                println!(
                    "skipping: installed utils package has no aliases in {}",
                    library.display()
                );
                continue;
            };
            let raw = database.raw_topic(&canonical).unwrap();
            let document = rd_ast::lower_r_object(&raw).unwrap();
            let provider = InstalledDocumentationProvider::new(std::slice::from_ref(&library));
            let projected = provider
                .get_topic(&TopicRequest::External {
                    package: "utils".into(),
                    topic: TopicRef(alias.clone()),
                })
                .unwrap()
                .expect("alias should resolve to an installed topic");

            assert_eq!(
                provider.topic_exists("utils", &alias),
                TopicExistence::Known(true)
            );
            assert_eq!(
                provider.topic_exists("utils", "mini_roxygen_alias_that_does_not_exist_73b91e"),
                TopicExistence::Known(false)
            );

            assert_eq!(
                projected.identity,
                DocumentationIdentity::External {
                    package: "utils".into(),
                    topic: canonical.clone(),
                }
            );
            assert_eq!(
                provider
                    .get_topic(&TopicRequest::External {
                        package: "utils".into(),
                        topic: TopicRef("mini_roxygen_topic_that_does_not_exist_9f3a7c".into(),),
                    })
                    .unwrap(),
                None
            );
            match document.title() {
                Some(nodes) => assert_eq!(
                    projected
                        .fields
                        .title
                        .as_ref()
                        .map(|content| &content.value),
                    Some(&InheritableContent::Rd(nodes.to_vec()))
                ),
                None => assert!(projected.fields.title.is_none()),
            }

            let canonical_request = TopicRequest::External {
                package: "utils".into(),
                topic: TopicRef(canonical),
            };
            assert_eq!(
                provider.get_topic(&canonical_request).unwrap(),
                Some(projected)
            );
            assert_eq!(provider.databases.borrow().len(), 1);
            assert_eq!(provider.topics.borrow().len(), 1);
        }
    }

    #[test]
    fn installed_help_provider_reaches_external_inheritance_when_available() {
        let libraries = installed_r_libraries();
        if libraries.is_empty() {
            println!("skipping: no standard installed R library is available");
            return;
        }

        for library in libraries {
            if !library.join("utils/Meta/package.rds").is_file() {
                println!(
                    "skipping: installed utils package is unavailable in {}",
                    library.display()
                );
                continue;
            }
            let mut sources = SourceMap::new();
            sources.add_file(SourceFile::new(
                PathBuf::from("R/inherit.R"),
                r#"#' @name target
#' @inherit utils::person title
target <- function() NULL

#' @name mean_target
#' @title Mean target
#' @param x Local x
#' @param na.rm Local na.rm
#' @inheritParams base::mean trim
mean_target <- function(x, trim = 0, na.rm = FALSE) NULL
"#
                .to_owned(),
            ));
            let inputs = PackageInputs {
                sources,
                metadata: PackageMetadata::new("currentPackage", None).unwrap(),
            };
            let options = DocumentOptions {
                inline_r_substitutions: mini_roxygen_core::InlineRSubstitutions::builtins()
                    .unwrap(),
                s3_registrars: Default::default(),
            };
            let inheritance_options = InheritanceOptions {
                external: ExternalInheritancePolicy::BestEffort,
                external_source: ExternalPolicySource::Explicit,
            };
            let provider = InstalledDocumentationProvider::new(std::slice::from_ref(&library));
            let output = document_package_with_options_and_providers(
                &inputs,
                &options,
                &EmptyS3GenericProvider,
                &provider,
                &inheritance_options,
            );
            let target = output
                .rd
                .files
                .get(&TopicKey("target".into()))
                .expect("external target Rd should be generated");
            insta::allow_duplicates! {
                insta::assert_snapshot!(target.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/inherit.R
\name{target}
\alias{target}
\title{Persons}
\usage{
target()
}
\description{
Persons
}
"###);
            }
            assert!(!output.diagnostics().any(|diagnostic| {
                diagnostic.code == mini_roxygen_core::DiagnosticCode::UnresolvedInherit
            }));
            let mean_target = output
                .rd
                .files
                .get(&TopicKey("mean_target".into()))
                .expect("external parameter target Rd should be generated");
            insta::allow_duplicates! {
                insta::assert_snapshot!(mean_target.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/inherit.R
\name{mean_target}
\alias{mean_target}
\title{Mean target}
\usage{
mean_target(x, trim = 0, na.rm = FALSE)
}
\arguments{
\item{x}{Local x}

\item{trim}{the fraction (0 to 0.5) of observations to be
    trimmed from each end of \code{x} before the mean is computed.
    Values of trim outside that range are taken as the nearest endpoint.
  }

\item{na.rm}{Local na.rm}
}
\description{
Mean target
}
"###);
            }
        }
    }

    #[test]
    fn installed_rlang_inheritance_absolutizes_donor_relative_links_when_available() {
        let libraries = installed_r_libraries();
        if libraries.is_empty() {
            println!("skipping: no installed R library is available");
            return;
        }

        let mut tested = false;
        for library in libraries {
            if !library.join("rlang/Meta/package.rds").is_file() {
                println!(
                    "skipping: installed rlang package is unavailable in {}",
                    library.display()
                );
                continue;
            }
            tested = true;
            let mut sources = SourceMap::new();
            sources.add_file(SourceFile::new(
                PathBuf::from("R/inherit-rlang.R"),
                r#"#' @name target
#' @title Target
#' @inheritParams rlang::args_error_context call
target <- function(call = NULL) NULL
"#
                .to_owned(),
            ));
            let inputs = PackageInputs {
                sources,
                metadata: PackageMetadata::new("recipient", None).unwrap(),
            };
            let options = DocumentOptions {
                inline_r_substitutions: mini_roxygen_core::InlineRSubstitutions::builtins()
                    .unwrap(),
                s3_registrars: Default::default(),
            };
            let inheritance_options = InheritanceOptions {
                external: ExternalInheritancePolicy::BestEffort,
                external_source: ExternalPolicySource::Explicit,
            };
            let provider = InstalledDocumentationProvider::new(std::slice::from_ref(&library));
            let output = document_package_with_options_and_providers(
                &inputs,
                &options,
                &EmptyS3GenericProvider,
                &provider,
                &inheritance_options,
            );
            let target = output
                .rd
                .files
                .get(&TopicKey("target".into()))
                .expect("rlang parameter target Rd should be generated");
            assert!(target.content.contains(r"\link[rlang:abort]{abort()}"));
            assert!(!target.content.contains(r"\link[=abort]"));
        }

        if !tested {
            println!("skipping: rlang is unavailable in discovered R libraries");
        }
    }
}
