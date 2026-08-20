//! Exercises the public source-to-Rd pipeline against a package on disk.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use arity_parser::ast::Expr;
use arity_parser::parser::{ParseOptions, parse_with_options};
use arity_parser::syntax::{SyntaxElement, SyntaxKind};
use mini_roxygen_core::{
    Diagnostic, DiagnosticCode, FileId, PackageInputs, PackageOutput, Severity, SourceMap,
    TopicKey, document_package,
};
use rd_ast::{RdDocument, RdNode, RdTag};
use serde_json::{Value, json};

fn fixture_package_root(package: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/packages")
        .join(package)
}

fn assert_r_accepts_batch<'a>(documents: impl IntoIterator<Item = (&'a str, &'a str)>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/parse-rd-batch.R")
        .canonicalize()
        .expect("the batch Rd oracle script is available");
    let workspace = tempfile::tempdir().expect("the Rd oracle has a temporary workspace");
    let mut paths = Vec::new();
    for (label, document) in documents {
        let path = workspace.path().join(format!("{label}.Rd"));
        fs::write(&path, document).expect("the Rd oracle document is writable");
        paths.push(path);
    }
    let output = match Command::new("Rscript")
        .arg("--vanilla")
        .arg(script)
        .args(&paths)
        .output()
    {
        Ok(output) => output,
        Err(error) if std::env::var_os("MINI_ROXYGEN_REQUIRE_RD_ORACLE").is_none() => {
            eprintln!("Rd oracle skipped: {error}");
            return;
        }
        Err(error) => panic!("Rscript could not be run: {error}"),
    };
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line == "STATUS ok"),
        "R rejected generated Rd:\n{}\n{}",
        paths
            .iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .collect::<Vec<_>>()
            .join("\n"),
        String::from_utf8_lossy(&output.stdout)
    );
}

fn assert_namespace_accepts(fixture: &str, package: &str, document: &str) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/parse-namespace.R")
        .canonicalize();
    let script = match script {
        Ok(script) => script,
        Err(error) if std::env::var_os("MINI_ROXYGEN_REQUIRE_RD_ORACLE").is_none() => {
            eprintln!("NAMESPACE oracle skipped: {error}");
            return;
        }
        Err(error) => panic!("NAMESPACE oracle script is unavailable: {error}"),
    };
    let library = tempfile::tempdir().expect("NAMESPACE oracle library is writable");
    let package_root = library.path().join(package);
    fs::create_dir(&package_root).expect("NAMESPACE oracle package directory is writable");
    fs::copy(
        fixture_package_root(fixture).join("DESCRIPTION"),
        package_root.join("DESCRIPTION"),
    )
    .expect("NAMESPACE oracle DESCRIPTION is readable");
    fs::write(package_root.join("NAMESPACE"), document).expect("NAMESPACE oracle file is writable");
    let output = match Command::new("Rscript")
        .arg("--vanilla")
        .arg(script)
        .arg(package)
        .arg(library.path())
        .output()
    {
        Ok(output) => output,
        Err(error) if std::env::var_os("MINI_ROXYGEN_REQUIRE_RD_ORACLE").is_none() => {
            eprintln!("NAMESPACE oracle skipped: {error}");
            return;
        }
        Err(error) => panic!("Rscript could not run the NAMESPACE oracle: {error}"),
    };
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line == "STATUS ok"),
        "R rejected generated NAMESPACE:\n{}\n{}",
        document,
        String::from_utf8_lossy(&output.stdout)
    );
}

fn assert_pkgdown_extracts_sources(document: &str, expected: &[&str]) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/pkgdown-extract-source.R")
        .canonicalize();
    let script = match script {
        Ok(script) => script,
        Err(error) if std::env::var_os("MINI_ROXYGEN_REQUIRE_RD_ORACLE").is_none() => {
            eprintln!("pkgdown source oracle skipped: {error}");
            return;
        }
        Err(error) => panic!("pkgdown source oracle is unavailable: {error}"),
    };
    let workspace = tempfile::tempdir().expect("the pkgdown source oracle has a workspace");
    let path = workspace.path().join("document.Rd");
    fs::write(&path, document).expect("the pkgdown source oracle document is writable");
    let output = match Command::new("Rscript")
        .arg("--vanilla")
        .arg(script)
        .arg(path)
        .output()
    {
        Ok(output) => output,
        Err(error) if std::env::var_os("MINI_ROXYGEN_REQUIRE_RD_ORACLE").is_none() => {
            eprintln!("pkgdown source oracle skipped: {error}");
            return;
        }
        Err(error) => panic!("Rscript could not run the pkgdown source oracle: {error}"),
    };
    assert!(
        output.status.success(),
        "pkgdown source oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("SOURCE "))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert!(
        stdout.lines().any(|line| line == "STATUS ok"),
        "pkgdown source oracle produced no successful verdict:\n{}",
        stdout
    );
}

fn build_fixture(package: &str) -> (PackageInputs, PackageOutput) {
    let inputs = PackageInputs::from_package_root(fixture_package_root(package))
        .expect("the fixture package should load");
    let output = document_package(&inputs);
    (inputs, output)
}

/// Renders a path with forward slashes whatever the platform separator is.
///
/// The generator itself already writes slash-joined paths into the generation
/// header, so only this harness would otherwise compare a native separator
/// against a slash-separated expectation.
fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn render_diagnostics<'a>(
    diagnostics: impl Iterator<Item = &'a Diagnostic>,
    sources: &SourceMap,
) -> String {
    let mut lines = diagnostics
        .map(|diagnostic| format_diagnostic(diagnostic, sources))
        .collect::<Vec<_>>();
    lines.sort();
    lines.join("\n")
}

fn collect_tags(node: &RdNode, tags: &mut Vec<RdTag>) {
    match node {
        RdNode::Tagged(tagged) => {
            if !tags.contains(tagged.tag()) {
                tags.push(tagged.tag().clone());
            }
            for child in tagged.children() {
                collect_tags(child, tags);
            }
        }
        RdNode::Group(group) => {
            for child in group.children() {
                collect_tags(child, tags);
            }
        }
        RdNode::Comment(_)
        | RdNode::Text(_)
        | RdNode::RCode(_)
        | RdNode::Verb(_)
        | RdNode::Raw(_) => {}
        _ => {}
    }
}

fn format_diagnostic(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    let source = sources
        .get(diagnostic.primary.span.file)
        .expect("diagnostic points to a registered source")
        .path();
    let range = diagnostic.primary.span.range;
    format!(
        "{} {} {}[{}..{}]: {}",
        severity_name(diagnostic.severity),
        diagnostic.code.as_str(),
        slash_path(source),
        range.start(),
        range.end(),
        diagnostic.message
    )
}

fn ast_text(nodes: &[RdNode]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            RdNode::Text(value) | RdNode::RCode(value) | RdNode::Verb(value) => {
                text.push_str(value);
            }
            RdNode::Comment(_) => {}
            RdNode::Tagged(tagged) => text.push_str(&ast_text(tagged.children())),
            RdNode::Group(group) => text.push_str(&ast_text(group.children())),
            RdNode::Raw(raw) => text.push_str(&ast_text(raw.children())),
            _ => {}
        }
    }
    text
}

fn normalized_topic(document: &RdDocument) -> Value {
    let name = document.name().map(ast_text).unwrap_or_default();
    let mut aliases = document
        .nodes()
        .iter()
        .filter_map(|node| match node {
            RdNode::Tagged(tagged) if tagged.tag() == &RdTag::Alias => {
                Some(ast_text(tagged.children()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut parameters = document
        .arguments()
        .flat_map(|argument| {
            ast_text(argument.name)
                .split(',')
                .map(|name| name.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let usage = document
        .usage()
        .map(ast_text)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "));
    aliases.sort();
    aliases.dedup();
    parameters.sort();
    parameters.dedup();
    json!({
        "name": name,
        "aliases": aliases,
        "parameters": parameters,
        "usage": usage,
    })
}

fn is_namespace_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

fn canonical_namespace_expression(element: SyntaxElement) -> String {
    let mut canonical = String::new();
    let tokens = match element {
        SyntaxElement::Node(node) => node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .collect::<Vec<_>>(),
        SyntaxElement::Token(token) => vec![token],
    };
    for token in tokens {
        if !is_namespace_trivia(token.kind()) {
            canonical.push_str(token.text());
        }
    }
    canonical
}

fn normalized_namespace(content: &str) -> Vec<String> {
    let parsed = parse_with_options(content, &ParseOptions::default());
    assert!(
        parsed.diagnostics.is_empty(),
        "generated NAMESPACE must parse without diagnostics: {:?}",
        parsed.diagnostics
    );
    let mut directives = parsed
        .cst
        .children_with_tokens()
        .filter_map(|element| {
            Expr::cast(element.clone()).map(|_| canonical_namespace_expression(element))
        })
        .collect::<Vec<_>>();
    directives.sort();
    directives.dedup();
    directives
}

#[test]
fn namespace_semantics_preserve_spaces_inside_quoted_arguments() {
    assert_eq!(
        normalized_namespace("export(\"a b\")\n"),
        vec!["export(\"a b\")"]
    );
}

fn normalized_semantics(output: &PackageOutput, notes: &[&str]) -> Value {
    let topics = output
        .rd
        .files
        .values()
        .map(|file| normalized_topic(&file.document))
        .collect::<Vec<_>>();
    json!({
        "topics": topics,
        "namespace_directives": normalized_namespace(&output.namespace.content),
        "notes": notes,
    })
}

fn assert_semantics_fixture(package: &str, output: &PackageOutput, notes: &[&str]) {
    let expected_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/expected")
        .join(package)
        .join("semantics.json");
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(expected_path).expect("semantics fixture is readable"),
    )
    .expect("semantics fixture is valid JSON");
    assert_eq!(normalized_semantics(output, notes), expected);
}

fn normalized_diagnostics(output: &PackageOutput, sources: &SourceMap) -> Value {
    let mut diagnostics = output
        .diagnostics()
        .map(|diagnostic| {
            json!({
                "code": diagnostic.code.as_str(),
                "severity": severity_name(diagnostic.severity),
                "file": slash_path(sources.get(diagnostic.primary.span.file).expect("diagnostic source").path()),
                "start": diagnostic.primary.span.range.start(),
                "end": diagnostic.primary.span.range.end(),
            })
        })
        .collect::<Vec<_>>();
    diagnostics.sort_by_key(Value::to_string);
    Value::Array(diagnostics)
}

#[test]
fn package_fixture_generates_the_expected_rd_files() {
    let (inputs, output) = build_fixture("rd-basic");
    let actual_paths = output
        .rd
        .files
        .values()
        .map(|file| slash_path(&file.relative_path))
        .collect::<BTreeSet<_>>();
    let expected_paths = [
        "man/basic.Rd",
        "man/print.fixture.Rd",
        "man/shared.Rd",
        "man/suppressed.Rd",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    // The exact path set catches a topic silently appearing or disappearing,
    // even when an existing topic's content snapshot remains unchanged.
    assert_eq!(
        actual_paths,
        expected_paths,
        "diagnostics:\n{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    assert_eq!(
        output.rd.files[&TopicKey("shared".to_owned())].relative_path,
        Path::new("man/shared.Rd")
    );

    // Each inline snapshot is the complete writer output for its model topic,
    // so a changed header, section, or usage is visible beside the relevant topic.
    for (topic, generated) in &output.rd.files {
        match topic.as_str() {
            "basic" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-basic.R
\name{basic}
\alias{basic}
\title{Basic fixture title.}
\usage{
basic(x, y)
}
\arguments{
\item{x, y}{The two values to add.}
}
\value{
The combined value.
}
\description{
This is the \strong{description} with \code{inline_code} and a \url{https://example.com/guide} link.
}
\details{
These details keep \emph{emphasis} and explain the documented calculation.

A second details paragraph makes the implicit intro genuinely multi-paragraph.
}
\note{
This note is retained in the generated Rd.
}
\section{More information}{
 This named section is part of the fixture.
}

\examples{
result <- basic(1, 2)
}
\references{
A fixture reference.
}
\seealso{
The \url{https://stat.ethz.ch/R-manual/R-devel/library/base/html/base-package.html} base package.
}
\author{
The mini-roxygen maintainers.
}
\keyword{fixtures}
\keyword{functions}
"#),
            "print.fixture" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/03-shared-second.R
\name{print.fixture}
\alias{print.fixture}
\title{Print method fixture.}
\usage{
\method{print}{fixture}(x, ...)
}
\description{
This topic contributes a statically generated S3 method usage.
}
"#),
            "shared" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-shared-first.R, R/03-shared-second.R
\name{shared_generated}
\alias{shared_generated}
\alias{shared_explicit}
\title{Shared fixture topic.}
\usage{
shared_generated(value)

shared_explicit(value, mode = "fast")
}
\arguments{
\item{value}{The value passed to the shared topic.}

\item{mode}{The mode for the explicit shared call.}
}
\description{
The first contribution supplies the shared title and description.
}
"#),
            "suppressed" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-shared-first.R
\name{suppressed}
\alias{suppressed}
\title{Suppressed usage fixture.}
\description{
Suppressed usage fixture.
}
"#),
            other => panic!("unexpected rd-basic topic: {other}"),
        }
        let expected = match topic.as_str() {
            "basic" => &["R/01-basic.R"][..],
            "print.fixture" => &["R/03-shared-second.R"][..],
            "shared" => &["R/02-shared-first.R", "R/03-shared-second.R"][..],
            "suppressed" => &["R/02-shared-first.R"][..],
            other => panic!("unexpected rd-basic topic: {other}"),
        };
        assert_pkgdown_extracts_sources(&generated.content, expected);
    }
    assert_r_accepts_batch(
        output
            .rd
            .files
            .iter()
            .map(|(topic, file)| (topic.0.as_str(), file.content.as_str())),
    );

    // Rendering resolves FileId to the package-relative path and sorts complete
    // lines, so diagnostics never expose registration numbers or map order.
    let rendered = render_diagnostics(output.diagnostics(), &inputs.sources);
    insta::assert_snapshot!("diagnostics", rendered);
}

#[test]
fn markdown_fixture_generates_the_expected_rd_files() {
    let (inputs, output) = build_fixture("markdown-rd");
    let actual_paths = output
        .rd
        .files
        .values()
        .map(|file| slash_path(&file.relative_path))
        .collect::<BTreeSet<_>>();
    let expected_paths = [
        "man/markdown_aliasless.Rd",
        "man/markdown_section.Rd",
        "man/markdown_table.Rd",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);

    for (topic, generated) in &output.rd.files {
        let expected = match topic.as_str() {
            "markdown_aliasless" => &["R/03-aliasless.R"][..],
            "markdown_section" => &["R/02-section.R"][..],
            "markdown_table" => &["R/01-table.R"][..],
            other => panic!("unexpected markdown-rd topic: {other}"),
        };
        assert_pkgdown_extracts_sources(&generated.content, expected);
    }
    assert_r_accepts_batch(
        output
            .rd
            .files
            .iter()
            .map(|(topic, file)| (topic.0.as_str(), file.content.as_str())),
    );

    let rendered = render_diagnostics(output.diagnostics(), &inputs.sources);
    insta::assert_snapshot!("markdown_rd__diagnostics", rendered);

    let aliasless = output
        .rd
        .files
        .values()
        .find(|file| file.relative_path == Path::new("man/markdown_aliasless.Rd"))
        .expect("the alias-less Markdown topic is generated");
    insta::assert_snapshot!(aliasless.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/03-aliasless.R
\name{markdown_aliasless}
\title{Alias-less Markdown fixture.}
\usage{
markdown_aliasless()
}
\description{
Alias-less Markdown fixture.
}
"#);

    let table = output
        .rd
        .files
        .values()
        .find(|file| file.relative_path == Path::new("man/markdown_table.Rd"))
        .expect("the Markdown table topic is generated");
    insta::assert_snapshot!(table.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-table.R
\name{markdown_table}
\alias{markdown_table}
\title{Markdown table fixture.}
\usage{
markdown_table()
}
\description{
\tabular{lr}{
   Name \tab Value \cr
   \strong{alpha} \tab \code{x + 1} \cr
}
}
"#);

    let section = output
        .rd
        .files
        .values()
        .find(|file| file.relative_path == Path::new("man/markdown_section.Rd"))
        .expect("the Markdown section topic is generated");
    insta::assert_snapshot!(section.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-section.R
\name{markdown_section}
\alias{markdown_section}
\title{Markdown section fixture.}
\usage{
markdown_section()
}
\description{
Markdown section fixture.
}
\section{Details}{
 This section has \strong{strong} text.
\itemize{
\item outer item
\itemize{
\item inner item
}
}
}

\examples{
markdown_section()
}
"#);

    let failing_file = (0..inputs.sources.len())
        .map(|index| {
            inputs
                .sources
                .get(FileId::new(
                    u32::try_from(index).expect("fixture has few files"),
                ))
                .expect("registered source file")
        })
        .find(|file| file.path() == Path::new("R/04-failing.R"))
        .expect("the failing fixture source is registered");
    let offending = failing_file
        .text()
        .find("`r 1 + 1`")
        .expect("the failing Markdown is present");
    let failing_diagnostic = output
        .diagnostics()
        .find(|diagnostic| diagnostic.code.as_str() == "undefined-inline-r-substitution")
        .expect("inline R conversion emits an error diagnostic");
    assert_eq!(failing_diagnostic.severity, Severity::Error);
    assert_eq!(failing_diagnostic.primary.span.file, FileId::new(3));
    assert_eq!(
        failing_diagnostic.primary.span.range.start(),
        u32::try_from(offending).expect("fixture offset fits u32")
    );
    assert_eq!(
        failing_diagnostic.primary.span.range.end(),
        u32::try_from(offending + "`r 1 + 1`".len()).expect("fixture offset fits u32")
    );
}

#[test]
fn syntax_fixture_covers_source_and_standalone_topics() {
    let (inputs, output) = build_fixture("syntax");
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    assert_semantics_fixture(
        "syntax",
        &output,
        &["markdown_config_false_is_ignored_and_markdown_is_always_enabled"],
    );
    let expected_topics = [
        "RenderOptions",
        "label_style_scheme",
        "lambda_fixture",
        "raw_rd_fixture",
        "signature_fixture",
        "standalone_fixture",
    ];
    assert_eq!(
        output
            .rd
            .files
            .keys()
            .map(|key| key.0.as_str())
            .collect::<Vec<_>>(),
        expected_topics
    );
    let expected_paths = [
        "man/RenderOptions.Rd",
        "man/lambda_fixture.Rd",
        "man/label_style_scheme.Rd",
        "man/raw_rd_fixture.Rd",
        "man/signature_fixture.Rd",
        "man/standalone_fixture.Rd",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let actual_paths = output
        .rd
        .files
        .values()
        .map(|file| slash_path(&file.relative_path))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);
    for (topic, generated) in &output.rd.files {
        match topic.0.as_str() {
            "RenderOptions" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/05-s7.R
\name{RenderOptions}
\alias{RenderOptions}
\alias{new_render_options}
\title{Synthetic renderer options.}
\usage{
new_render_options(..., compact = TRUE)
}
\arguments{
\item{...}{Additional constructor arguments.}

\item{compact}{Whether compact rendering is enabled.}
}
\description{
Synthetic renderer options.
}
"#),
            "lambda_fixture" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-lambda.R
\name{lambda_fixture}
\alias{lambda_fixture}
\title{Shorthand lambda fixture.}
\usage{
lambda_fixture(x)
}
\arguments{
\item{x}{The input value.}
}
\value{
The input value.
}
\description{
Shorthand lambda fixture.
}
"#),
            "label_style_scheme" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/05-s7.R
\name{new_label_style}
\alias{new_label_style}
\alias{LabelStyle}
\alias{label_style_scheme}
\title{Synthetic label styling.}
\usage{
new_label_style(template, ..., separator = NULL)
}
\arguments{
\item{template}{The label template.}

\item{separator}{The separator between label parts.}
}
\description{
Synthetic label styling.
}
"#),
            "raw_rd_fixture" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/04-raw-rd.R
\name{raw_rd_fixture}
\alias{raw_rd_fixture}
\title{Raw Rd fixture.}
\usage{
raw_rd_fixture()
}
\description{
Raw Rd equation: \eqn{x^2}{x^2}.
}
"#),
            "signature_fixture" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-signature.R
\name{signature_fixture}
\alias{signature_fixture}
\title{Multiline signature fixture.}
\usage{
signature_fixture(
  x,
  y = \{
    x + 1
  \},
  ...
)
}
\arguments{
\item{x, y}{The values to combine.}

\item{...}{Additional arguments.}
}
\value{
The combined value.
}
\description{
This description uses \strong{Markdown} even though the package config disables it.
}
"#),
            "standalone_fixture" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/03-standalone.R
\name{standalone_fixture}
\alias{standalone_fixture}
\alias{standalone_alias}
\title{Standalone topic fixture.}
\description{
A topic documented without a function body.
}
"#),
            other => panic!("unexpected syntax fixture topic: {other}"),
        }
    }
    assert_r_accepts_batch(
        output
            .rd
            .files
            .iter()
            .map(|(topic, file)| (topic.0.as_str(), file.content.as_str())),
    );
}

#[test]
fn local_inheritance_fixture_covers_recursive_params() {
    let (inputs, output) = build_fixture("inherit-local");
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    assert_semantics_fixture("inherit-local", &output, &[]);
    for (topic, generated) in &output.rd.files {
        match topic.0.as_str() {
            "inherit_donor" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-donor.R
\name{inherit_donor}
\alias{inherit_donor}
\title{Inheritance donor.}
\usage{
inherit_donor(x, y)
}
\arguments{
\item{x}{The first donor value.}

\item{y}{The second donor value.}
}
\description{
Inheritance donor.
}
"#),
            "inherit_middle" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-middle.R
\name{inherit_middle}
\alias{inherit_middle}
\title{Intermediate inheritance topic.}
\usage{
inherit_middle(x, y)
}
\arguments{
\item{x}{The first donor value.}

\item{y}{The second donor value.}
}
\description{
Intermediate inheritance topic.
}
"#),
            "inherit_target" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/03-target.R
\name{inherit_target}
\alias{inherit_target}
\title{Recursive inheritance target.}
\usage{
inherit_target(x, y)
}
\arguments{
\item{x}{The first donor value.}

\item{y}{The second donor value.}
}
\description{
Recursive inheritance target.
}
"#),
            other => panic!("unexpected inheritance fixture topic: {other}"),
        }
    }
    assert_r_accepts_batch(
        output
            .rd
            .files
            .iter()
            .map(|(topic, file)| (topic.0.as_str(), file.content.as_str())),
    );
}

#[test]
fn namespace_fixture_covers_exports_and_merged_imports() {
    let (inputs, output) = build_fixture("namespace");
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    assert_semantics_fixture("namespace", &output, &[]);
    insta::assert_snapshot!(output.namespace.content, @r#"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand

export(explicit_export_name)
export(implicit_export)
import(stats)
importFrom(utils,
  head,
  match,
  tail
)
    "#);
    for (topic, generated) in &output.rd.files {
        match topic.0.as_str() {
            "explicit_export" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-exports.R
\name{explicit_export}
\alias{explicit_export}
\title{Explicit export fixture.}
\usage{
explicit_export(x)
}
\description{
Explicit export fixture.
}
"#),
            "implicit_export" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/01-exports.R
\name{implicit_export}
\alias{implicit_export}
\title{Implicit export fixture.}
\usage{
implicit_export(x)
}
\description{
Implicit export fixture.
}
"#),
            "namespace_match" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/03-merged-import.R
\name{namespace_match}
\alias{namespace_match}
\title{A second importFrom contribution for the same package.}
\usage{
namespace_match(x, table)
}
\description{
A second importFrom contribution for the same package.
}
"#),
            "namespace_target" => insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/02-imports.R
\name{namespace_target}
\alias{namespace_target}
\title{Imported function fixture.}
\usage{
namespace_target(x)
}
\description{
Imported function fixture.
}
"#),
            other => panic!("unexpected namespace fixture topic: {other}"),
        }
    }
    assert_r_accepts_batch(
        output
            .rd
            .files
            .iter()
            .map(|(topic, file)| (topic.0.as_str(), file.content.as_str())),
    );
    assert_namespace_accepts("namespace", "namespacefixture", &output.namespace.content);
}

#[test]
fn diagnostics_fixture_keeps_failures_separate_from_success_fixtures() {
    let (inputs, output) = build_fixture("diagnostics");
    let codes = output
        .diagnostics()
        .map(|diagnostic| diagnostic.code)
        .collect::<BTreeSet<_>>();
    assert!(codes.contains(&DiagnosticCode::InheritCycle));
    assert!(codes.contains(&DiagnosticCode::MissingParam));
    assert!(codes.contains(&DiagnosticCode::UndefinedInlineRSubstitution));
    let expected_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/expected/diagnostics/diagnostics.json");
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(expected_path).expect("diagnostics fixture is readable"),
    )
    .expect("diagnostics fixture is valid JSON");
    assert_eq!(normalized_diagnostics(&output, &inputs.sources), expected);
    insta::assert_snapshot!(render_diagnostics(output.diagnostics(), &inputs.sources), @r#"
error inherit-cycle R/01-cycle.R[25..59]: inheritance cycle: diagnostic_cycle_a -> diagnostic_cycle_b -> diagnostic_cycle_a
error undefined-inline-r-substitution R/03-inline-r.R[67..76]: no substitution is defined for inline R expression
warning missing-param R/02-missing-param.R[132..144]: parameter `undocumented` is not documented
"#);
}

#[test]
fn package_fixture_generates_a_package_topic() {
    let (inputs, output) = build_fixture("package-rd");
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    let generated = output
        .rd
        .files
        .get(&TopicKey("package.rd-package".to_owned()))
        .expect("package topic should be generated");
    insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/package.R
\docType{package}
\name{package.rd-package}
\alias{package.rd}
\alias{package.rd-package}
\title{package.rd: A Package Topic}
\description{
A package description.
}
\seealso{
Useful links:
\itemize{\item\url{https://example.org}
\item\url{https://example.org/docs}
\item\url{https://example.org/project}
\item Report bugs at \url{https://example.org/issues}
}
}
\author{
\strong{Maintainer}: Ada Lovelace \email{ada@example.org}



Authors:
\itemize{\item Ada Lovelace \email{ada@example.org}
\item Grace Hopper (\href{https://orcid.org/0000-0000-0000-0000}{ORCID})
\item The Example Project Contributors
}

Other contributors:
\itemize{\item Example Foundation [copyright holder, funder]
}
}
\keyword{internal}
"#);
    assert_pkgdown_extracts_sources(&generated.content, &["R/package.R"]);
    let mut tags = Vec::new();
    for node in generated.document.nodes() {
        collect_tags(node, &mut tags);
    }
    for tag in [
        RdTag::Strong,
        RdTag::Email,
        RdTag::Href,
        RdTag::Itemize,
        RdTag::Item,
        RdTag::Url,
    ] {
        assert!(tags.contains(&tag), "missing structured Rd tag {tag:?}");
    }
    assert_r_accepts_batch([("package", generated.content.as_str())]);
}

#[test]
fn package_explicit_fields_override_defaults_and_description_null_suppresses_fallback() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: explicit.pkg\nTitle: Default title\nDescription: Default description\nURL: https://example.org\nBugReports: https://example.org/issues\nAuthors@R: person(\"Default\", \"Author\", role = \"aut\")\n",
    )
    .expect("DESCRIPTION should be writable");
    fs::write(
        root.path().join("R/package.R"),
        "#' @description NULL\n#' @author Explicit author\n#' @seealso Explicit seealso\n\"_PACKAGE\"\n",
    )
    .expect("package source should be writable");
    let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let output = document_package(&inputs);
    assert!(!output.has_errors());
    let generated = output
        .rd
        .files
        .get(&TopicKey("explicit.pkg-package".to_owned()))
        .expect("package topic should be generated");
    insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/package.R
\docType{package}
\name{explicit.pkg-package}
\alias{explicit.pkg}
\alias{explicit.pkg-package}
\title{explicit.pkg: Default title}
\seealso{
Explicit seealso
}
\author{
Explicit author
}
"#);
}

#[test]
fn malformed_authors_warn_but_package_rd_and_namespace_continue() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: malformed.pkg\nTitle: Malformed Authors\nDescription: Package description\nAuthors@R: foo(\"Broken\")\n",
    )
    .expect("DESCRIPTION should be writable");
    fs::write(
        root.path().join("R/package.R"),
        "#' @keywords internal\n\"_PACKAGE\"\n",
    )
    .expect("package source should be writable");
    let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let output = document_package(&inputs);
    assert!(!output.has_errors());
    assert_eq!(
        output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::PackageAuthorsParse)
            .count(),
        1
    );
    let generated = output
        .rd
        .files
        .get(&TopicKey("malformed.pkg-package".to_owned()))
        .expect("package topic should be generated");
    insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/package.R
\docType{package}
\name{malformed.pkg-package}
\alias{malformed.pkg}
\alias{malformed.pkg-package}
\title{malformed.pkg: Malformed Authors}
\description{
Package description
}
\keyword{internal}
"#);
    insta::assert_snapshot!(output.namespace.content, @r#"
# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand

"#);
}

#[test]
fn package_fallback_suppression_controls_rd_generation_and_description() {
    for (source, expected_rd, expected_diagnostic) in [
        (
            "#' @keywords internal\n#' @title NULL\n\"_PACKAGE\"\n",
            false,
            Some(DiagnosticCode::MissingPackageTitle),
        ),
        (
            "#' @keywords internal\n#' @title Explicit title\n#' @description NULL\n\"_PACKAGE\"\n",
            true,
            None,
        ),
    ] {
        let root = tempfile::tempdir().expect("temporary package root");
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        fs::write(
            root.path().join("DESCRIPTION"),
            "Package: suppression.pkg\nTitle: Default title\nVersion: 0.1.0\nDescription: Default description.\nAuthors@R: person(\"Default\", \"Author\", role = \"aut\")\n",
        )
        .expect("DESCRIPTION should be writable");
        fs::write(root.path().join("R/package.R"), source)
            .expect("package source should be writable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        let output = document_package(&inputs);
        assert_eq!(
            output
                .diagnostics()
                .filter(|diagnostic| Some(diagnostic.code) == expected_diagnostic)
                .count(),
            if expected_diagnostic.is_some() { 1 } else { 0 }
        );
        assert_eq!(
            output
                .diagnostics()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingPackageTitle)
                .count(),
            usize::from(expected_diagnostic == Some(DiagnosticCode::MissingPackageTitle))
        );
        let topic_key = TopicKey("suppression.pkg-package".to_owned());
        assert_eq!(output.rd.files.contains_key(&topic_key), expected_rd);
        if expected_rd {
            let generated = output.rd.files.get(&topic_key).expect("package Rd file");
            insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/package.R
\docType{package}
\name{suppression.pkg-package}
\alias{suppression.pkg}
\alias{suppression.pkg-package}
\title{Explicit title}
\author{


Authors:
\itemize{\item Default Author
}
}
\keyword{internal}
"#);
        }
    }
}

#[test]
fn inherited_structured_fields_override_package_structured_fallbacks() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: inherited.pkg\nTitle: Default title\nVersion: 0.1.0\nDescription: Default description.\nURL: https://example.org\nBugReports: https://example.org/issues\nAuthors@R: person(\"Default\", \"Author\", role = \"aut\")\n",
    )
    .expect("DESCRIPTION should be writable");
    fs::write(
        root.path().join("R/topics.R"),
        r#"#' @title Donor title
#' @seealso Inherited seealso
#' @author Inherited author
donor <- function() donor
#' @inherit donor seealso author
#' @name inherited.pkg-package
NULL
#' @keywords internal
"_PACKAGE"
"#,
    )
    .expect("package source should be writable");
    let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let output = document_package(&inputs);
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    let generated = output
        .rd
        .files
        .get(&TopicKey("inherited.pkg-package".to_owned()))
        .expect("package topic should be generated");
    insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{inherited.pkg-package}
\alias{inherited.pkg-package}
\alias{inherited.pkg}
\title{inherited.pkg: Default title}
\description{
Default description.
}
\seealso{
Inherited seealso
}
\author{
Inherited author
}
\keyword{internal}
"#);
}

#[test]
fn inherited_package_fields_resolve_before_metadata_diagnostics() {
    for (description, inherited, _expected) in [
        (
            "Package: inherited.pkg\nVersion: 0.1.0\nDescription: Package description.\nAuthors@R: person(\"Default\", \"Author\", role = \"aut\")\n",
            "title",
            "Donor title",
        ),
        (
            "Package: inherited.pkg\nTitle: Package title\nVersion: 0.1.0\nAuthors@R: person(\"Default\", \"Author\", role = \"aut\")\n",
            "description",
            "Donor description.",
        ),
        (
            "Package: inherited.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\nAuthors@R: foo(\"Broken\")\n",
            "author",
            "Donor author",
        ),
    ] {
        let root = tempfile::tempdir().expect("temporary package root");
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        fs::write(root.path().join("DESCRIPTION"), description)
            .expect("DESCRIPTION should be writable");
        fs::write(
            root.path().join("R/topics.R"),
            format!(
                "#' @title Donor title\n#' @description Donor description.\n#' @author Donor author\ndonor <- function() donor\n#' @inherit donor {inherited}\n#' @name inherited.pkg-package\nNULL\n#' @keywords internal\n\"_PACKAGE\"\n"
            ),
        )
        .expect("R source should be writable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        let output = document_package(&inputs);
        assert!(
            !output.has_errors(),
            "{}",
            render_diagnostics(output.diagnostics(), &inputs.sources)
        );
        assert_eq!(
            output
                .diagnostics()
                .filter(|diagnostic| matches!(
                    diagnostic.code,
                    DiagnosticCode::MissingPackageTitle
                        | DiagnosticCode::MissingPackageDescription
                        | DiagnosticCode::PackageAuthorsParse
                ))
                .count(),
            0
        );
        let generated = output
            .rd
            .files
            .get(&TopicKey("inherited.pkg-package".to_owned()))
            .expect("package topic should be generated");
        match inherited {
            "title" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{inherited.pkg-package}
\alias{inherited.pkg-package}
\alias{inherited.pkg}
\title{Donor title}
\description{
Package description.
}
\author{


Authors:
\itemize{\item Default Author
}
}
\keyword{internal}
"#),
            "description" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{inherited.pkg-package}
\alias{inherited.pkg-package}
\alias{inherited.pkg}
\title{inherited.pkg: Package title}
\description{
Donor description.
}
\author{


Authors:
\itemize{\item Default Author
}
}
\keyword{internal}
"#),
            "author" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{inherited.pkg-package}
\alias{inherited.pkg-package}
\alias{inherited.pkg}
\title{inherited.pkg: Package title}
\description{
Package description.
}
\author{
Donor author
}
\keyword{internal}
"#),
            other => panic!("unexpected inherited field: {other}"),
        }
    }
}

#[test]
fn package_inheritance_and_local_values_beat_metadata_fallbacks() {
    for (source, description, expected, _absent) in [
        (
            "#' @title NULL\n#' @inherit donor title\n#' @name precedence.pkg-package\nNULL\n",
            "Package: precedence.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\n",
            "Donor title",
            "Package title",
        ),
        (
            "#' @description NULL\n#' @inherit donor description\n#' @name precedence.pkg-package\nNULL\n",
            "Package: precedence.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\n",
            "Donor description.",
            "Package description.",
        ),
        (
            "#' @seealso NULL\n#' @inherit donor seealso\n#' @name precedence.pkg-package\nNULL\n",
            "Package: precedence.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\nURL: https://example.org\nBugReports: https://example.org/issues\n",
            "Donor seealso",
            "https://example.org",
        ),
        (
            "#' @author NULL\n#' @inherit donor author\n#' @name precedence.pkg-package\nNULL\n",
            "Package: precedence.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\nAuthors@R: foo(\"Broken\")\n",
            "Donor author",
            "failed to parse Authors@R",
        ),
        (
            "#' @title Local title\n#' @inherit donor title\n#' @name precedence.pkg-package\nNULL\n",
            "Package: precedence.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\n",
            "Local title",
            "Donor title",
        ),
    ] {
        let root = tempfile::tempdir().expect("temporary package root");
        fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
        fs::write(root.path().join("DESCRIPTION"), description)
            .expect("DESCRIPTION should be writable");
        fs::write(
            root.path().join("R/topics.R"),
            format!(
                "#' @title Donor title\n#' @description Donor description.\n#' @seealso Donor seealso\n#' @author Donor author\ndonor <- function() donor\n{source}#' @keywords internal\n\"_PACKAGE\"\n",
            ),
        )
        .expect("R source should be writable");
        let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
        let output = document_package(&inputs);
        assert!(
            !output.has_errors(),
            "{}",
            render_diagnostics(output.diagnostics(), &inputs.sources)
        );
        let generated = output
            .rd
            .files
            .get(&TopicKey("precedence.pkg-package".to_owned()))
            .expect("package topic should be generated");
        match expected {
            "Donor title" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{precedence.pkg-package}
\alias{precedence.pkg-package}
\alias{precedence.pkg}
\title{Donor title}
\description{
Package description.
}
\keyword{internal}
"#),
            "Donor description." => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{precedence.pkg-package}
\alias{precedence.pkg-package}
\alias{precedence.pkg}
\title{precedence.pkg: Package title}
\description{
Donor description.
}
\keyword{internal}
"#),
            "Donor seealso" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{precedence.pkg-package}
\alias{precedence.pkg-package}
\alias{precedence.pkg}
\title{precedence.pkg: Package title}
\description{
Package description.
}
\seealso{
Donor seealso
}
\keyword{internal}
"#),
            "Donor author" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{precedence.pkg-package}
\alias{precedence.pkg-package}
\alias{precedence.pkg}
\title{precedence.pkg: Package title}
\description{
Package description.
}
\author{
Donor author
}
\keyword{internal}
"#),
            "Local title" => insta::assert_snapshot!(&generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\docType{package}
\name{precedence.pkg-package}
\alias{precedence.pkg-package}
\alias{precedence.pkg}
\title{Local title}
\description{
Package description.
}
\keyword{internal}
"#),
            other => panic!("unexpected precedence field: {other}"),
        }
        assert_eq!(
            output
                .diagnostics()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::PackageAuthorsParse)
                .count(),
            0
        );
    }
}

#[test]
fn package_metadata_diagnostics_use_one_package_anchor_and_missing_package_title() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: missing.pkg\nVersion: 0.1.0\nAuthors@R: foo(\"Broken\")\n",
    )
    .expect("DESCRIPTION should be writable");
    let source = "#' @keywords internal\n\"_PACKAGE\"\n";
    fs::write(root.path().join("R/package.R"), source).expect("R source should be writable");
    let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let output = document_package(&inputs);
    assert!(
        !output
            .rd
            .files
            .contains_key(&TopicKey("missing.pkg-package".to_owned(),))
    );
    assert_eq!(
        output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingPackageTitle)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingTopicTitle)
            .count(),
        0
    );
    assert_eq!(
        output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingPackageDescription)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::PackageAuthorsParse)
            .count(),
        1
    );
    let metadata_diagnostics = output
        .diagnostics()
        .filter(|diagnostic| {
            matches!(
                diagnostic.code,
                DiagnosticCode::MissingPackageTitle
                    | DiagnosticCode::MissingPackageDescription
                    | DiagnosticCode::PackageAuthorsParse
            )
        })
        .collect::<Vec<_>>();
    assert!(metadata_diagnostics.iter().all(|diagnostic| {
        diagnostic.primary.span == metadata_diagnostics[0].primary.span
            && source[diagnostic.primary.span.range.start() as usize
                ..diagnostic.primary.span.range.end() as usize]
                .contains("_PACKAGE")
    }));
    assert_eq!(
        metadata_diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == DiagnosticCode::MissingPackageTitle)
            .expect("missing package title diagnostic")
            .message,
        "package topic `missing.pkg-package` has no title"
    );
}

#[test]
fn package_metadata_fallbacks_remain_visible_when_package_topic_is_a_donor() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: donor.pkg\nTitle: Package title\nVersion: 0.1.0\nDescription: Package description.\n",
    )
    .expect("DESCRIPTION should be writable");
    fs::write(
        root.path().join("R/topics.R"),
        "#' @inherit donor.pkg title description\n#' @name consumer\nNULL\n#' @keywords internal\n\"_PACKAGE\"\n",
    )
    .expect("R source should be writable");
    let inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let output = document_package(&inputs);
    assert!(
        !output.has_errors(),
        "{}",
        render_diagnostics(output.diagnostics(), &inputs.sources)
    );
    let generated = output
        .rd
        .files
        .get(&TopicKey("consumer".to_owned()))
        .expect("consumer topic should be generated");
    insta::assert_snapshot!(generated.content, @r#"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/topics.R
\name{consumer}
\alias{consumer}
\title{donor.pkg: Package title}
\description{
Package description.
}

"#);
}
