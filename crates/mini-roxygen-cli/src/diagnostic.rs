use mini_roxygen_core::{Diagnostic, Severity, SourceMap, Span};

use crate::output::{OutputErrors, WriteFailure};

pub(crate) fn render_diagnostics<'a>(
    sources: &SourceMap,
    diagnostics: impl Iterator<Item = &'a Diagnostic>,
) -> String {
    diagnostics
        .map(|diagnostic| render_diagnostic(diagnostic, sources))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_diagnostic(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    let mut lines = vec![format!(
        "{}: {}[{}]: {}",
        render_location(sources, diagnostic.primary.span),
        severity_name(diagnostic.severity),
        diagnostic.code.as_str(),
        diagnostic.message
    )];

    lines.extend(diagnostic.secondary.iter().map(|label| {
        format!(
            "  note: {}: {}",
            render_location(sources, label.span),
            label.message
        )
    }));
    if let Some(help) = &diagnostic.help {
        lines.push(format!("  help: {help}"));
    }
    lines.join("\n")
}

fn render_location(sources: &SourceMap, span: Span) -> String {
    match sources.span_location(span) {
        Some((path, line, column)) => format!("{}:{line}:{column}", path.display()),
        None => "<unknown>".to_owned(),
    }
}

const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

pub(crate) fn render_output_errors(errors: &OutputErrors) -> String {
    errors
        .errors
        .iter()
        .map(|error| {
            let code = error
                .stable_code()
                .map(|code| format!("[{code}]"))
                .unwrap_or_default();
            format!("error{code}: {error}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_write_failure(failure: &WriteFailure) -> String {
    let completed = render_paths(failure.completed.created.iter())
        .into_iter()
        .chain(render_paths(failure.completed.replaced.iter()))
        .chain(render_paths(failure.completed.unchanged.iter()))
        .collect::<Vec<_>>();
    let not_attempted = render_paths(failure.not_attempted.iter());
    format!(
        "error: {error}\n  completed: {completed}\n  not attempted: {not_attempted}",
        error = failure.error,
        completed = if completed.is_empty() {
            "none".to_owned()
        } else {
            completed.join(", ")
        },
        not_attempted = if not_attempted.is_empty() {
            "none".to_owned()
        } else {
            not_attempted.join(", ")
        },
    )
}

fn render_paths<'a>(paths: impl Iterator<Item = &'a std::path::PathBuf>) -> Vec<String> {
    paths.map(|path| path.display().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use mini_roxygen_core::{
        Diagnostic, DiagnosticCode, DocumentOptions, FileId, InlineRSubstitutions, Label,
        PackageInputs, PackageMetadata, Severity, SourceFile, SourceMap, Span, TextRange,
        document_package_with_options,
    };

    use super::render_diagnostics;

    fn span(file: u32, start: u32, end: u32) -> Span {
        Span::new(FileId::new(file), TextRange::new(start, end))
    }

    #[test]
    fn renders_primary_secondary_and_help() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/foo.R"),
            "first\n@param x\n".to_owned(),
        ));
        sources.add_file(SourceFile::new(
            PathBuf::from("R/bar.R"),
            "earlier\n".to_owned(),
        ));
        let diagnostic = Diagnostic::new(
            Severity::Error,
            DiagnosticCode::ConflictingParamDescription,
            "parameter has multiple descriptions",
            Label::new(span(0, 6, 12), "current description"),
        )
        .with_secondary(Label::new(span(1, 0, 1), "earlier description is here"))
        .with_help("keep one description for this parameter");

        let rendered = render_diagnostics(&sources, std::iter::once(&diagnostic));
        assert!(rendered.contains(
            "R/foo.R:2:1: error[conflicting-param-description]: parameter has multiple descriptions"
        ));
        assert!(rendered.contains("  note: R/bar.R:1:1: earlier description is here"));
        assert!(rendered.contains("  help: keep one description for this parameter"));
    }

    #[test]
    fn unresolved_spans_use_a_non_panicking_fallback() {
        let sources = SourceMap::new();
        let diagnostic = Diagnostic::new(
            Severity::Warning,
            DiagnosticCode::UnknownTag,
            "unknown tag @foo",
            Label::new(span(9, 0, 1), "unknown tag"),
        );

        assert_eq!(
            render_diagnostics(&sources, std::iter::once(&diagnostic)),
            "<unknown>: warning[unknown-tag]: unknown tag @foo"
        );
    }

    #[test]
    fn configuration_diagnostics_do_not_point_at_an_r_source_file() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/a.R"),
            "value <- function() NULL\n".to_owned(),
        ));

        let invalid = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("broken()".to_owned(), r#"\strong{"#.to_owned())]),
            Some("mini-roxygen.toml".to_owned()),
        )
        .expect_err("invalid Rd should produce diagnostics");
        let rendered = render_diagnostics(&sources, invalid.iter());
        assert!(rendered.starts_with(
            "<unknown>: error[invalid-inline-r-substitution]: invalid Rd substitution"
        ));
        assert!(!rendered.contains("R/a.R"));

        let substitutions = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("never()".to_owned(), r#"\code{never}"#.to_owned())]),
            Some("mini-roxygen.toml".to_owned()),
        )
        .expect("valid substitution");
        let inputs = PackageInputs {
            sources: sources.clone(),
            metadata: PackageMetadata::new("example", None).expect("valid package"),
        };
        let output = document_package_with_options(
            &inputs,
            &DocumentOptions {
                inline_r_substitutions: substitutions,
                s3_registrars: Default::default(),
            },
        );
        let rendered = render_diagnostics(
            &sources,
            output
                .diagnostics()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnusedInlineRSubstitution),
        );
        assert!(rendered.starts_with(
            "<unknown>: warning[unused-inline-r-substitution]: inline R substitution"
        ));
        assert!(!rendered.contains("R/a.R"));
    }

    #[test]
    fn unmatched_null_s3_metadata_has_stable_rendered_output() {
        let mut sources = SourceMap::new();
        sources.add_file(SourceFile::new(
            PathBuf::from("R/register.R"),
            r#"#' @title Unmatched method
#' @exportS3Method NULL
some_method <- function(x) x
"#
            .to_owned(),
        ));
        let inputs = PackageInputs {
            sources: sources.clone(),
            metadata: PackageMetadata::new("example", None).expect("valid package"),
        };
        let output = document_package_with_options(
            &inputs,
            &DocumentOptions {
                inline_r_substitutions: InlineRSubstitutions::builtins()
                    .expect("built-in substitutions should be valid"),
                s3_registrars: Default::default(),
            },
        );
        let rendered = render_diagnostics(
            &sources,
            output
                .diagnostics()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedS3MethodMetadata),
        );
        insta::assert_snapshot!(rendered, @r###"
R/register.R:2:4: error[unresolved-s3-method-metadata]: @exportS3Method NULL has no statically known generic and class
  help: add a matching registrar configuration or an explicit @method <generic> <class>
"###);
    }
}
