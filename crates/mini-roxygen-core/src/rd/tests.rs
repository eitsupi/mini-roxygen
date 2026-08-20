use std::fs;
use std::path::PathBuf;

use rd_ast::{RdDocument, RdNode, RdTag};

use crate::arity_adapter::parse;
use crate::inherit::{
    DocumentationProvider, InheritableContent, InheritanceOptions, ResolvedPackageModel,
    resolve_inheritance,
};
use crate::model::test_support::model_with_tag_diagnostics_and_sources;
use crate::model::{
    BlockRef, DocumentedBlock, ModelOutput, PackageModel, TopicKey,
    build_package_model_with_bindings, build_package_model_with_metadata,
};
use crate::namespace::{S3GenericProvider, classify_usage_methods};
use crate::package::{PackageInputs, PackageMetadata};
use crate::r_parse::build_object_index;
use crate::source::{SourceFile, SourceMap};
use crate::tags::{TagParseOptions, UnknownTagPolicy, parse_block};

use super::build_rd;

fn document(text: &str, topic: &str) -> RdDocument {
    let (model_output, sources) = model(text);
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(
        !output.diagnostics.has_errors(),
        "generated topic should be valid: {:?}",
        output.diagnostics
    );
    output
        .files
        .get(&TopicKey(topic.to_owned()))
        .unwrap_or_else(|| panic!("generated topic {topic}: {:?}", output.diagnostics))
        .document
        .clone()
}

fn tagged_nodes<'a>(node: &'a RdNode, tag: &RdTag, result: &mut Vec<&'a rd_ast::RdTagged>) {
    match node {
        RdNode::Tagged(tagged) => {
            if tagged.tag() == tag {
                result.push(tagged);
            }
            if let Some(option) = tagged.option() {
                for child in option {
                    tagged_nodes(child, tag, result);
                }
            }
            for child in tagged.children() {
                tagged_nodes(child, tag, result);
            }
        }
        RdNode::Group(group) => {
            for child in group.children() {
                tagged_nodes(child, tag, result);
            }
        }
        RdNode::Raw(raw) => {
            if let Some(option) = raw.option() {
                for child in option {
                    tagged_nodes(child, tag, result);
                }
            }
            for child in raw.children() {
                tagged_nodes(child, tag, result);
            }
        }
        _ => {}
    }
}

fn top_level_tags(document: &RdDocument) -> Vec<RdTag> {
    document
        .nodes()
        .iter()
        .filter_map(|node| node.as_tagged().map(|tagged| tagged.tag().clone()))
        .collect()
}

fn assert_all_leaves_are_rcode(node: &RdNode) {
    match node {
        RdNode::RCode(_) => {}
        RdNode::Tagged(tagged) => {
            if let Some(option) = tagged.option() {
                for child in option {
                    assert_all_leaves_are_rcode(child);
                }
            }
            for child in tagged.children() {
                assert_all_leaves_are_rcode(child);
            }
        }
        RdNode::Group(group) => {
            for child in group.children() {
                assert_all_leaves_are_rcode(child);
            }
        }
        RdNode::Raw(raw) => {
            if let Some(option) = raw.option() {
                for child in option {
                    assert_all_leaves_are_rcode(child);
                }
            }
            for child in raw.children() {
                assert_all_leaves_are_rcode(child);
            }
        }
        other => panic!("unexpected non-RCode leaf in code macro: {other:?}"),
    }
}

fn assert_all_leaves_are_text(node: &RdNode) {
    match node {
        RdNode::Text(_) => {}
        RdNode::Tagged(tagged) => {
            if let Some(option) = tagged.option() {
                for child in option {
                    assert_all_leaves_are_text(child);
                }
            }
            for child in tagged.children() {
                assert_all_leaves_are_text(child);
            }
        }
        RdNode::Group(group) => {
            for child in group.children() {
                assert_all_leaves_are_text(child);
            }
        }
        RdNode::Raw(raw) => {
            if let Some(option) = raw.option() {
                for child in option {
                    assert_all_leaves_are_text(child);
                }
            }
            for child in raw.children() {
                assert_all_leaves_are_text(child);
            }
        }
        other => panic!("unexpected non-Text leaf in prose macro: {other:?}"),
    }
}

#[test]
fn generated_document_pins_section_and_leaf_shapes() {
    let (model_output, sources) = model(
        r#"#' @name shape
#' @aliases alpha beta
#' @title Title
#' @format Format
#' @source Source
#' @usage shape(x)
#' @param x Parameter
#' @return Value
#' @description Description
#' @details Details
#' @note Note
#' @section Extra: Body
#' @examples
#' x <- 1
#' @references References
#' @seealso See also
#' @author Author
#' @keywords beta alpha
shape <- function(x) x
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = output.files.get(&TopicKey("shape".to_owned())).unwrap();

    assert_eq!(
        top_level_tags(&generated.document),
        vec![
            RdTag::Name,
            RdTag::Alias,
            RdTag::Alias,
            RdTag::Alias,
            RdTag::Title,
            RdTag::Format,
            RdTag::Source,
            RdTag::Usage,
            RdTag::Arguments,
            RdTag::Value,
            RdTag::Description,
            RdTag::Details,
            RdTag::Note,
            RdTag::Section,
            RdTag::Examples,
            RdTag::References,
            RdTag::SeeAlso,
            RdTag::Author,
            RdTag::Keyword,
            RdTag::Keyword,
        ]
    );
    for tag in [RdTag::Name, RdTag::Alias] {
        let mut nodes = Vec::new();
        for node in generated.document.nodes() {
            tagged_nodes(node, &tag, &mut nodes);
        }
        assert!(!nodes.is_empty());
        for node in nodes {
            assert_eq!(node.children().len(), 1);
            assert!(matches!(node.children()[0], RdNode::Verb(_)));
        }
    }

    let mut sections = Vec::new();
    for node in generated.document.nodes() {
        tagged_nodes(node, &RdTag::Section, &mut sections);
    }
    let section = sections.as_slice().first().expect("named section");
    assert_eq!(section.children().len(), 2);
    assert!(
        section
            .children()
            .iter()
            .all(|node| node.as_group().is_some())
    );
    assert_eq!(
        section.children()[0].as_group().unwrap().children(),
        &[RdNode::Text("Extra".to_owned())]
    );
    assert_eq!(
        section.children()[1].as_group().unwrap().children(),
        &[
            RdNode::Text("\n".to_owned()),
            RdNode::Text(" Body\n".to_owned()),
        ]
    );

    let mut items = Vec::new();
    for node in generated.document.nodes() {
        tagged_nodes(node, &RdTag::Item, &mut items);
    }
    assert_eq!(items.len(), 1);
    for item in items {
        assert_eq!(item.children().len(), 2);
        assert!(item.children().iter().all(|node| node.as_group().is_some()));
    }

    for tag in [RdTag::Usage, RdTag::Examples] {
        let mut nodes = Vec::new();
        for node in generated.document.nodes() {
            tagged_nodes(node, &tag, &mut nodes);
        }
        assert_eq!(nodes.len(), 1);
        for node in nodes {
            assert_all_leaves_are_rcode(&RdNode::Tagged(node.clone()));
        }
    }

    for tag in [
        RdTag::Title,
        RdTag::Format,
        RdTag::Source,
        RdTag::Arguments,
        RdTag::Value,
        RdTag::Description,
        RdTag::Details,
        RdTag::Note,
        RdTag::Section,
        RdTag::References,
        RdTag::SeeAlso,
        RdTag::Author,
        RdTag::Keyword,
    ] {
        let mut nodes = Vec::new();
        for node in generated.document.nodes() {
            tagged_nodes(node, &tag, &mut nodes);
        }
        assert!(!nodes.is_empty(), "missing prose macro {tag:?}");
        for node in nodes {
            assert_all_leaves_are_text(&RdNode::Tagged(node.clone()));
        }
    }
}

#[test]
fn local_multi_name_and_backtick_parameter_labels_remain_plain() {
    let (model_output, sources) = model(
        r#"#' @name parameter_labels
#' @title Parameter labels
#' @param x,y Shared description
#' @param `x,y` Comma-named description
parameter_labels <- function(x, y, `x,y`) NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("parameter_labels".to_owned()))
        .expect("generated parameter label topic");
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(generated.content, @r###"
% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
% Please edit documentation in R/test.R
\name{parameter_labels}
\alias{parameter_labels}
\title{Parameter labels}
\usage{
parameter_labels(x, y, `x,y`)
}
\arguments{
\item{x, y}{Shared description}

\item{x,y}{Comma-named description}
}
\description{
Parameter labels
}
"###);
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn one_seealso_markdown_list_lowers_to_one_section() {
    let (model_output, sources) = model(
        r#"#' @title Seealso list
#' @seealso
#' - [first()] is the first contribution.
#' - [second()] is the second contribution.
f <- function() f
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = output.files.get(&TopicKey("f".to_owned())).expect("f.Rd");

    insta::assert_snapshot!(&generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{f}
        \alias{f}
        \title{Seealso list}
        \usage{
        f()
        }
        \description{
        Seealso list
        }
        \seealso{
        \itemize{
        \item \code{\link[=first]{first()}} is the first contribution.
        \item \code{\link[=second]{second()}} is the second contribution.
        }
        }
        ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn itemize_items_take_no_arguments() {
    // `\item` is two-argument only under `\arguments` and `\describe`. In a
    // list its content follows it as siblings, the way roxygen2 writes it.
    let list_document = document(
        r#"#' @name list
#' @title List
#' @description
#' - first
#' - second
list <- function() {}
"#,
        "list",
    );
    let mut itemizes = Vec::new();
    for node in list_document.nodes() {
        tagged_nodes(node, &RdTag::Itemize, &mut itemizes);
    }
    assert_eq!(itemizes.len(), 1);

    let mut list_items = Vec::new();
    for node in list_document.nodes() {
        tagged_nodes(node, &RdTag::Item, &mut list_items);
    }
    assert_eq!(list_items.len(), 2);
    for item in list_items {
        assert!(item.children().is_empty());
        assert!(item.option().is_none());
    }

    let rendered = itemizes[0]
        .children()
        .iter()
        .filter_map(|node| match node {
            RdNode::Text(value) => Some(value.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        rendered.contains("first") && rendered.contains("second"),
        "item content is carried by siblings, not arguments: {rendered:?}"
    );
}

#[test]
fn usage_states_that_contribute_nothing_are_missing_not_empty() {
    let (model_output, sources) = model(
        r#"#' @name absent
#' @aliases NULL
#' @title Absent
NULL

#' @name suppressed
#' @title Suppressed
#' @usage NULL
suppressed <- function(x) x

#' @name suppressed-mixed
#' @title Suppressed mixed
#' @usage NULL
first <- function() first
#' @name suppressed-mixed
#' @usage first()
second <- function() second
#' @name suppressed-mixed
#' @usage second()
third <- function() third

#' @name absent-mixed
#' @aliases NULL
#' @title Absent mixed
NULL
#' @name absent-mixed
#' @usage survivor()
survivor <- function() survivor
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    let absent = output.files[&TopicKey("absent".to_owned())]
        .document
        .clone();
    assert_eq!(
        top_level_tags(&absent),
        vec![RdTag::Name, RdTag::Title, RdTag::Description]
    );
    assert!(
        absent
            .nodes()
            .iter()
            .all(|node| !matches!(node.as_tagged(), Some(tagged) if tagged.tag() == &RdTag::Usage))
    );

    let suppressed = output.files[&TopicKey("suppressed".to_owned())]
        .document
        .clone();
    assert!(
        suppressed
            .nodes()
            .iter()
            .all(|node| !matches!(node.as_tagged(), Some(tagged) if tagged.tag() == &RdTag::Usage))
    );

    let suppressed_mixed = output.files[&TopicKey("suppressed-mixed".to_owned())]
        .document
        .clone();
    let usage = suppressed_mixed
        .nodes()
        .iter()
        .find_map(|node| node.as_tagged().filter(|node| node.tag() == &RdTag::Usage))
        .expect("surviving usage contributions produce a usage macro");
    assert_eq!(
        usage.children(),
        &[
            RdNode::RCode("\n".to_owned()),
            RdNode::RCode("first()\n".to_owned()),
            RdNode::RCode("\n".to_owned()),
            RdNode::RCode("second()\n".to_owned()),
        ]
    );

    let absent_mixed = output.files[&TopicKey("absent-mixed".to_owned())]
        .document
        .clone();
    let usage = absent_mixed
        .nodes()
        .iter()
        .find_map(|node| node.as_tagged().filter(|node| node.tag() == &RdTag::Usage))
        .expect("the present contribution produces a usage macro");
    assert_eq!(
        usage.children(),
        &[
            RdNode::RCode("\n".to_owned()),
            RdNode::RCode("survivor()\n".to_owned()),
        ]
    );
}

struct NoExternalProvider;

impl DocumentationProvider for NoExternalProvider {
    fn get_topic(
        &self,
        _request: &crate::inherit::TopicRequest,
    ) -> Result<Option<crate::inherit::InheritableTopic>, crate::inherit::DocumentationError> {
        Ok(None)
    }
}

struct InstalledGenericProvider;

impl S3GenericProvider for InstalledGenericProvider {
    fn is_s3_generic(&self, name: &str) -> bool {
        name == "remote.alpha"
    }
}

fn resolved(package: &PackageModel) -> ResolvedPackageModel {
    resolve_inheritance(
        package,
        None,
        &super::EmptyLinks,
        &NoExternalProvider,
        &InheritanceOptions::default(),
    )
    .package
}

fn model(text: &str) -> (ModelOutput, SourceMap) {
    model_with_sources(&[("R/test.R", text)])
}

fn model_with_sources(files: &[(&str, &str)]) -> (ModelOutput, SourceMap) {
    let mut sources = SourceMap::new();
    let mut blocks = Vec::new();
    let mut bindings = Vec::new();
    for (path, text) in files {
        let source = SourceFile::new(PathBuf::from(path), (*text).to_owned());
        let file = sources.add_file(source.clone());
        let parsed = parse(&source, file);
        let index = build_object_index(parsed, file);
        bindings.extend(index.bindings.clone());
        let parsed = parse(&source, file);
        blocks.extend(index.documented.into_iter().map(|object| {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("indexed object has documentation");
            let (tags, diagnostics) = parse_block(
                &source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
            );
            assert!(diagnostics.is_empty(), "test tags should parse");
            DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            }
        }));
    }
    (
        build_package_model_with_bindings(&sources, blocks, bindings),
        sources,
    )
}

#[test]
fn builds_all_supported_fields_in_roxygen_section_order() {
    let (model_output, sources) = model(
        r"#' @name f
#' @aliases z a %foo% z
#' @title Title
#' @format Format
#' @source Source
#' @param x Parameter
#' @return Value
#' @description Description
#' @details Details
#' @note Note
#' @section Extra: Body
#' @examples
#' x <- 1
#' @references References
#' @seealso See also
#' @author Author
#' @keywords z a z
#' @usage f(x)
f <- function(x) x
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(
        !output.diagnostics.has_errors(),
        "generated topic should be valid: {:?}",
        output.diagnostics
    );
    let generated = output.files.get(&TopicKey("f".to_owned())).expect("f.Rd");
    let content = &generated.content;
    // The whole document is the assertion: section order, aliases in input
    // order against sorted keywords, and a `%` escaped inside an alias.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{f}
        \alias{f}
        \alias{z}
        \alias{a}
        \alias{\%foo\%}
        \title{Title}
        \format{
        Format
        }
        \source{
        Source
        }
        \usage{
        f(x)
        }
        \arguments{
        \item{x}{Parameter}
        }
        \value{
        Value
        }
        \description{
        Description
        }
        \details{
        Details
        }
        \note{
        Note
        }
        \section{Extra}{
         Body
        }

        \examples{
        x <- 1
        }
        \references{
        References
        }
        \seealso{
        See also
        }
        \author{
        Author
        }
        \keyword{a}
        \keyword{z}
        ");
}

#[test]
fn data_objects_emit_data_type_usage_and_dataset_keyword() {
    let (model_output, sources) = model(
        r#"#' Dataset title
"dataset"
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = output.files.get(&TopicKey("dataset".to_owned())).unwrap();
    insta::assert_snapshot!(&generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \docType{data}
        \name{dataset}
        \alias{dataset}
        \title{Dataset title}
        \usage{
        data(dataset)
        }
        \description{
        Dataset title
        }
        \keyword{datasets}
        ");
}

#[test]
fn data_topics_without_format_warn_at_the_missing_format_anchor() {
    let source = r#"#' Dataset title
"dataset"
"#;
    let (model_output, sources) = model(source);
    let output = build_rd(&resolved(&model_output.package), &sources);
    let warning = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat)
        .expect("missing data format warning");
    assert!(!output.diagnostics.has_errors());
    assert_eq!(warning.severity, crate::diagnostic::Severity::Warning);
    assert_eq!(
        warning.primary.span.range.start(),
        source.find("\"dataset\"").expect("data object") as u32
    );
}

#[test]
fn explicit_or_suppressed_data_format_does_not_warn() {
    for format in ["Format", "NULL"] {
        let source = format!("#' Dataset title\n#' @format {format}\n\"dataset\"\n");
        let (model_output, sources) = model(&source);
        let output = build_rd(&resolved(&model_output.package), &sources);
        assert!(!output.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
        }));
    }
}

#[test]
fn an_unformatted_data_contribution_still_warns_after_a_data_null_format() {
    let source = r#"#' Dataset title
#' @rdname shared
#' @format NULL
"dataset"
#' @rdname shared
"other_dataset"
"#;
    let (model_output, sources) = model(source);
    let output = build_rd(&resolved(&model_output.package), &sources);
    let warning = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat)
        .expect("missing data format warning");
    assert_eq!(
        warning.primary.span.range.start(),
        source
            .find("\"other_dataset\"")
            .expect("unformatted data object") as u32
    );
}

#[test]
fn an_ordinary_null_format_does_not_suppress_a_data_format_warning() {
    let (model_output, sources) = model(
        r#"#' Ordinary title
#' @rdname shared
#' @format NULL
ordinary <- 1
#' @rdname shared
"dataset"
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
    }));
}

#[test]
fn all_data_null_formats_do_not_warn() {
    let (model_output, sources) = model(
        r#"#' Dataset title
#' @rdname shared
#' @format NULL
"dataset"
#' @rdname shared
#' @format NULL
"other_dataset"
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
    }));
}

#[test]
fn an_explicit_data_format_satisfies_all_data_contributions() {
    let (model_output, sources) = model(
        r#"#' Dataset title
#' @rdname shared
#' @format Format
"dataset"
#' @rdname shared
"other_dataset"
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
    }));
}

#[test]
fn inherited_data_format_does_not_warn_after_resolution() {
    let (model_output, sources) = model(
        r#"#' Donor title
#' @format Donor format
donor <- 1
#' Dataset title
#' @inherit donor format
"dataset"
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
    }));
}

#[test]
fn conflicting_package_and_data_topics_do_not_add_missing_format_warning() {
    let mut sources = SourceMap::new();
    let source = r#"#' @title Package title
#' @rdname shared
"_PACKAGE"
#' @rdname shared
"dataset"
"#;
    let blocks = crate::model::test_support::blocks(&mut sources, "R/test.R", source);
    let metadata = PackageMetadata::new("example", None).expect("package metadata is valid");
    let model = build_package_model_with_metadata(&sources, blocks, &metadata);
    assert!(model.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::ConflictingTopicKind
    }));
    let output = build_rd(&resolved(&model.package), &sources);
    assert!(!output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == crate::diagnostic::DiagnosticCode::MissingDataFormat
    }));
}

#[test]
fn data_object_methods_do_not_lower_data_usage_as_s3_methods() {
    for (lazy_data, expected_usage) in [(false, "data(dataset)"), (true, "dataset")] {
        let mut sources = SourceMap::new();
        let source = r#"#' Dataset title
#' @method generic class
"dataset"
"#;
        let blocks = crate::model::test_support::blocks(&mut sources, "R/test.R", source);
        let metadata = PackageMetadata::new("example", None)
            .expect("package metadata is valid")
            .with_lazy_data(lazy_data);
        let model = build_package_model_with_metadata(&sources, blocks, &metadata);
        assert!(model.diagnostics.is_empty(), "{:?}", model.diagnostics);
        let output = build_rd(&resolved(&model.package), &sources);
        assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
        let content = &output.files[&TopicKey("dataset".to_owned())].content;
        assert!(content.contains(&format!("\\usage{{\n{expected_usage}\n}}")));
        assert!(!content.contains(r"\method{"));
    }
}

#[test]
fn refuses_invalid_topics_but_keeps_independent_topics() {
    let (model_output, sources) = model(
        r"#' @name missing
#' @inheritParams other
missing <- function() {}

#' @name good
#' @title Good
good <- function() {}
",
    );
    let inheritance = resolve_inheritance(
        &model_output.package,
        None,
        &super::EmptyLinks,
        &NoExternalProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        inheritance
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::UnresolvedInherit)
    );
    let output = build_rd(&inheritance.package, &sources);
    assert!(output.files.contains_key(&TopicKey("good".to_owned())));
    assert!(!output.files.contains_key(&TopicKey("missing".to_owned())));

    let (model, sources) = model(
        r"#' @name untitled
untitled <- function() {}
",
    );
    let output = build_rd(&resolved(&model.package), &sources);
    assert!(output.files.is_empty());
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingTopicTitle)
    );
}

#[test]
fn title_fragment_is_reused_for_the_description_fallback() {
    let (model, sources) = model(
        r"#' @name fallback
#' @title **Bold**
fallback <- function() {}
",
    );
    let output = build_rd(&resolved(&model.package), &sources);
    let content = &output
        .files
        .values()
        .next()
        .expect("fallback topic")
        .content;
    // The fallback copies the converted title, so the emphasis survives into
    // the description instead of raw Markdown reaching Rd.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{fallback}
        \alias{fallback}
        \title{\strong{Bold}}
        \usage{
        fallback()
        }
        \description{
        \strong{Bold}
        }
        ");
}

#[test]
fn generated_document_inherits_one_named_section() {
    let (model_output, sources) = model(
        r#"#' @name donor
#' @title Donor
#' @section Details: donor body
donor <- function() {}

#' @name target
#' @title Target
#' @inheritSection donor Details
target <- function() {}
"#,
    );
    let inheritance = resolve_inheritance(
        &model_output.package,
        None,
        &super::EmptyLinks,
        &NoExternalProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        !inheritance.diagnostics.has_errors(),
        "{:?}",
        inheritance.diagnostics
    );
    let output = build_rd(&inheritance.package, &sources);
    let content = &output.files[&TopicKey("target".to_owned())].content;
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{target}
        \alias{target}
        \title{Target}
        \usage{
        target()
        }
        \description{
        Target
        }
        \section{Details}{
         donor body
        }
        ");
}

#[test]
fn generated_document_deduplicates_semantic_local_sections_and_is_r_parseable() {
    let (model_output, sources) = model(
        r#"#' @name semantic
#' @title Semantic sections
#' @section A: first body
#' @section **A**: later body
semantic <- function() {}
"#,
    );
    let inheritance = resolve_inheritance(
        &model_output.package,
        None,
        &super::EmptyLinks,
        &NoExternalProvider,
        &InheritanceOptions::default(),
    );
    let conflicts = inheritance
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::ConflictingSectionTitle
        })
        .collect::<Vec<_>>();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].secondary.len(), 1);

    let output = build_rd(&inheritance.package, &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = &output.files[&TopicKey("semantic".to_owned())];
    insta::assert_snapshot!(generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{semantic}
        \alias{semantic}
        \title{Semantic sections}
        \usage{
        semantic()
        }
        \description{
        Semantic sections
        }
        \section{A}{
         first body
        }
        ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_is_accepted_by_r() {
    // The backslash-brace stress content lives inside a quoted string
    // deliberately: a bare backslash outside a string is only ever valid R
    // as `\(`, so a raw `\\{x\}` outside quotes is now diagnosed by the
    // examples raw-Rd scanner rather than passed through as escaping
    // stress. Moving it into a string literal keeps testing backslash and
    // brace escaping without relying on non-R-valid input. `%%` (a real,
    // closed modulo operator) keeps stressing `%` escaping instead of the
    // original bare, unclosed `%2`: this fixture must remain valid R so the
    // oracle can check the complete generated document. The adapter has a
    // focused boundary test for an unterminated percent line; this fixture's
    // closed `%%` keeps exercising percent escaping without conflating that
    // recovery case with R acceptance.
    let (model, sources) = model(
        r#"#' @name oracle
#' @title Oracle
#' @param x **value**
#' @examples
#' x <- 1 %% 2
#' y <- "\\{x}"
oracle <- function(x) x
"#,
    );
    let output = build_rd(&resolved(&model.package), &sources);
    let generated = &output
        .files
        .values()
        .next()
        .unwrap_or_else(|| panic!("oracle topic: {:?}", output.diagnostics))
        .content;
    crate::rd_oracle::assert_r_accepts(generated);
}

#[test]
fn generated_document_preserves_equation_commands() {
    let (model_output, sources) = model(
        r#"#' @name compute
#' @title Compute a value
#' @param value Input uses \eqn{\alpha}{alpha} and \deqn{\frac{x}{y}}{x over y}.
#' @return A result.
compute <- function(value) value
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("compute".to_owned()))
        .unwrap_or_else(|| panic!("compute topic: {:?}", output.diagnostics));

    insta::assert_snapshot!(generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{compute}
        \alias{compute}
        \title{Compute a value}
        \usage{
        compute(value)
        }
        \arguments{
        \item{value}{Input uses \eqn{\alpha}{alpha} and \deqn{\frac{x}{y}}{x over y}.}
        }
        \value{
        A result.
        }
        \description{
        Compute a value
        }
        ");
}

#[test]
fn generated_document_lowers_examples_if() {
    let (model_output, sources) = model(
        r#"#' @name conditional
#' @title Conditional examples
#' @examplesIf interactive()
#' value <- 1
conditional <- function(value) value
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("conditional".to_owned()))
        .unwrap_or_else(|| panic!("conditional topic: {:?}", output.diagnostics));

    insta::assert_snapshot!(generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{conditional}
        \alias{conditional}
        \title{Conditional examples}
        \usage{
        conditional(value)
        }
        \description{
        Conditional examples
        }
        \examples{
        \dontshow{if (\{
        interactive()
        \}) withAutoprint(\{ # examplesIf}
        value <- 1
        \dontshow{\}) # examplesIf}
        }
        ");
}

#[test]
fn generated_document_separates_condition_comments_from_wrappers() {
    let (model_output, sources) = model(
        r#"#' @name commented
#' @title Commented condition
#' @examplesIf interactive() # run interactively
#' value <- 1
commented <- function(value) value

#' @name terminated
#' @title Terminated condition
#' @examplesIf interactive() ;
#' value <- 2
terminated <- function(value) value
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);

    let commented = output
        .files
        .get(&TopicKey("commented".to_owned()))
        .unwrap_or_else(|| panic!("commented topic: {:?}", output.diagnostics));
    insta::assert_snapshot!(commented.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{commented}
        \alias{commented}
        \title{Commented condition}
        \usage{
        commented(value)
        }
        \description{
        Commented condition
        }
        \examples{
        \dontshow{if (\{
        interactive() # run interactively
        \}) withAutoprint(\{ # examplesIf}
        value <- 1
        \dontshow{\}) # examplesIf}
        }
        ");

    let terminated = output
        .files
        .get(&TopicKey("terminated".to_owned()))
        .unwrap_or_else(|| panic!("terminated topic: {:?}", output.diagnostics));
    insta::assert_snapshot!(terminated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{terminated}
        \alias{terminated}
        \title{Terminated condition}
        \usage{
        terminated(value)
        }
        \description{
        Terminated condition
        }
        \examples{
        \dontshow{if (\{
        interactive() ;
        \}) withAutoprint(\{ # examplesIf}
        value <- 2
        \dontshow{\}) # examplesIf}
        }
        ");
    crate::rd_oracle::assert_r_accepts(&commented.content);
    crate::rd_oracle::assert_r_accepts(&terminated.content);
}

#[test]
fn generated_document_lowers_dontrun_in_plain_examples() {
    // An ordinary example line, a blank line, then a multi-line \dontrun
    // body with a comment line, a `::` call, and a double-quoted string, and
    // a closing brace alone on its own line.
    let (model_output, sources) = model(
        r#"#' @name plot_summary
#' @title Plot a summary
#' @examples
#' summary <- summarize(data.frame(x = 1))
#' print(summary)
#'
#' \dontrun{
#' # requires an interactive graphics device
#' path <- save_plot(summary, file = "out.png")
#' grDevices::dev.off()
#' }
plot_summary <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("plot_summary".to_owned()))
        .unwrap_or_else(|| panic!("plot_summary topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r#"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{plot_summary}
    \alias{plot_summary}
    \title{Plot a summary}
    \usage{
    plot_summary()
    }
    \description{
    Plot a summary
    }
    \examples{
    summary <- summarize(data.frame(x = 1))
    print(summary)

    \dontrun{
    # requires an interactive graphics device
    path <- save_plot(summary, file = "out.png")
    grDevices::dev.off()
    }
    }
    "#);
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_lowers_raw_rd_macro_after_unterminated_percent() {
    let (model_output, sources) = model(
        r#"#' @name macro_after_percent
#' @title Macro after percent
#' @examples
#' value %
#' \dontshow{
#' hidden <- 1
#' }
macro_after_percent <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("macro_after_percent".to_owned()))
        .unwrap_or_else(|| panic!("macro_after_percent topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{macro_after_percent}
    \alias{macro_after_percent}
    \title{Macro after percent}
    \usage{
    macro_after_percent()
    }
    \description{
    Macro after percent
    }
    \examples{
    value \%
    \dontshow{
    hidden <- 1
    }
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_lowers_donttest_and_dontshow_in_plain_examples() {
    let (model_output, sources) = model(
        r#"#' @name skip_variants
#' @title Skip variants
#' @examples
#' before()
#' \donttest{
#' slow()
#' }
#' \dontshow{
#' setup()
#' }
#' after()
skip_variants <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("skip_variants".to_owned()))
        .unwrap_or_else(|| panic!("skip_variants topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{skip_variants}
    \alias{skip_variants}
    \title{Skip variants}
    \usage{
    skip_variants()
    }
    \description{
    Skip variants
    }
    \examples{
    before()
    \donttest{
    slow()
    }
    \dontshow{
    setup()
    }
    after()
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_lowers_testonly_and_dontdiff_in_plain_examples() {
    // `\testonly` and `\dontdiff` were missing from the supported set: R
    // accepts both as regular macros (confirmed against `tools::parse_Rd`;
    // both give their body the `RCODE` pseudo-tag, matching `\donttest` and
    // `\dontshow` rather than `\dontrun`'s `VERB`), so leaving them out was
    // a regression from the stronger "diagnose every non-`\(` backslash"
    // rule: a package author using either of them would previously get a
    // silently-broken Rd file, and now would incorrectly get an error for
    // valid R.
    let (model_output, sources) = model(
        r#"#' @name testonly_and_dontdiff
#' @title Testonly and dontdiff
#' @examples
#' before()
#' \testonly{
#' hidden_check()
#' }
#' \dontdiff{
#' nondeterministic_output()
#' }
#' after()
testonly_and_dontdiff <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("testonly_and_dontdiff".to_owned()))
        .unwrap_or_else(|| panic!("testonly_and_dontdiff topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{testonly_and_dontdiff}
    \alias{testonly_and_dontdiff}
    \title{Testonly and dontdiff}
    \usage{
    testonly_and_dontdiff()
    }
    \description{
    Testonly and dontdiff
    }
    \examples{
    before()
    \testonly{
    hidden_check()
    }
    \dontdiff{
    nondeterministic_output()
    }
    after()
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_lowers_a_dontshow_nested_inside_donttest() {
    let (model_output, sources) = model(
        r#"#' @name nested_examples
#' @title Nested examples
#' @examples
#' \donttest{
#' outer_call()
#' \dontshow{
#' inner_call()
#' }
#' after_inner()
#' }
nested_examples <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("nested_examples".to_owned()))
        .unwrap_or_else(|| panic!("nested_examples topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{nested_examples}
    \alias{nested_examples}
    \title{Nested examples}
    \usage{
    nested_examples()
    }
    \description{
    Nested examples
    }
    \examples{
    \donttest{
    outer_call()
    \dontshow{
    inner_call()
    }
    after_inner()
    }
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_keeps_macro_shaped_text_inside_dontrun_literal() {
    // Confirmed against `tools::parse_Rd`: R's own Rd parser does not
    // recognize a nested macro inside \dontrun's verbatim content, only
    // literal text, and rd-writer enforces the same rule structurally (a
    // Tagged child is rejected once the enclosing leaf mode is Verbatim).
    // So text shaped like \dontshow{...} written inside a \dontrun body
    // must stay literal, not become a nested tag -- and must not be
    // diagnosed either, since it is not being interpreted as a macro
    // attempt in that position at all.
    let (model_output, sources) = model(
        r#"#' @name dontshow_inside_dontrun
#' @title Dontshow inside dontrun
#' @examples
#' \dontrun{
#' a <- 1
#' \dontshow{
#' b <- 2
#' }
#' c <- 3
#' }
dontshow_inside_dontrun <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("dontshow_inside_dontrun".to_owned()))
        .unwrap_or_else(|| panic!("dontshow_inside_dontrun topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{dontshow_inside_dontrun}
    \alias{dontshow_inside_dontrun}
    \title{Dontshow inside dontrun}
    \usage{
    dontshow_inside_dontrun()
    }
    \description{
    Dontshow inside dontrun
    }
    \examples{
    \dontrun{
    a <- 1
    \\dontshow\{
    b <- 2
    \}
    c <- 3
    }
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_leaves_lambda_shorthand_untouched() {
    let (model_output, sources) = model(
        r#"#' @name lambda_shorthand
#' @title Lambda shorthand
#' @examples
#' f <- \(x) x + 1
#' f(1)
lambda_shorthand <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("lambda_shorthand".to_owned()))
        .unwrap_or_else(|| panic!("lambda_shorthand topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{lambda_shorthand}
    \alias{lambda_shorthand}
    \title{Lambda shorthand}
    \usage{
    lambda_shorthand()
    }
    \description{
    Lambda shorthand
    }
    \examples{
    f <- \\(x) x + 1
    f(1)
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_leaves_backslashes_inside_string_literals_untouched() {
    let (model_output, sources) = model(
        r#"#' @name string_escapes
#' @title String escapes
#' @examples
#' a <- "\\d"
#' b <- "\n"
string_escapes <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("string_escapes".to_owned()))
        .unwrap_or_else(|| panic!("string_escapes topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r#"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{string_escapes}
    \alias{string_escapes}
    \title{String escapes}
    \usage{
    string_escapes()
    }
    \description{
    String escapes
    }
    \examples{
    a <- "\\\\d"
    b <- "\\n"
    }
    "#);
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_does_not_miscount_braces_inside_strings_and_comments() {
    let (model_output, sources) = model(
        r#"#' @name brace_in_string
#' @title Brace in string
#' @examples
#' \dontshow{
#' x <- "a{b"
#' # a } comment
#' y <- 2
#' }
brace_in_string <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("brace_in_string".to_owned()))
        .unwrap_or_else(|| panic!("brace_in_string topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r#"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{brace_in_string}
    \alias{brace_in_string}
    \title{Brace in string}
    \usage{
    brace_in_string()
    }
    \description{
    Brace in string
    }
    \examples{
    \dontshow{
    x <- "a{b"
    # a \} comment
    y <- 2
    }
    }
    "#);
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_does_not_miscount_a_brace_inside_a_backtick_name() {
    // A backtick-quoted (non-syntactic) R name can contain a literal `{` or
    // `}`. `arity-parser`'s lexer emits a backtick name as one `IDENT`
    // token whose text keeps the backticks and any escaped content
    // verbatim, the same way a string literal is lexed as one `STRING`
    // token. The marker stream excludes backtick-quoted `IDENT` tokens, so
    // the `}` inside the name below cannot close the `\dontshow` brace early.
    let (model_output, sources) = model(
        r#"#' @name backtick_brace_in_dontshow
#' @title Backtick brace in dontshow
#' @examples
#' \dontshow{
#' `a}b` <- 1
#' y <- 2
#' }
backtick_brace_in_dontshow <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("backtick_brace_in_dontshow".to_owned()))
        .unwrap_or_else(|| panic!("backtick_brace_in_dontshow topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{backtick_brace_in_dontshow}
    \alias{backtick_brace_in_dontshow}
    \title{Backtick brace in dontshow}
    \usage{
    backtick_brace_in_dontshow()
    }
    \description{
    Backtick brace in dontshow
    }
    \examples{
    \dontshow{
    `a}b` <- 1
    y <- 2
    }
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_leaves_a_backslash_inside_a_backtick_name_untouched() {
    // Same lexing fact as the sibling test above, exercised at the top
    // level instead of inside a supported macro: a backslash inside a
    // backtick-quoted name is part of that one `IDENT` token, not an
    // independent, macro-shaped backslash, so it must not be diagnosed.
    let (model_output, sources) = model(
        r#"#' @name backtick_backslash_top_level
#' @title Backtick backslash top level
#' @examples
#' `a\tb` <- 1
backtick_backslash_top_level <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("backtick_backslash_top_level".to_owned()))
        .unwrap_or_else(|| {
            panic!(
                "backtick_backslash_top_level topic: {:?}",
                output.diagnostics
            )
        });
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{backtick_backslash_top_level}
    \alias{backtick_backslash_top_level}
    \title{Backtick backslash top level}
    \usage{
    backtick_backslash_top_level()
    }
    \description{
    Backtick backslash top level
    }
    \examples{
    `a\\tb` <- 1
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_leaves_a_nested_roxygen_marker_untouched() {
    // A `#'` line inside the *source* (not the normalized example text) is
    // stripped by the tag parser, so it never reaches this scanner -- but
    // writing the marker *twice* (`#' #' see \code{x}`) leaves one `#'`
    // behind in the normalized value, which then looks like a roxygen
    // comment line to `arity-parser` when this scanner re-parses it: the
    // lexer sub-tokenizes `#' see \code{x}` into `ROXYGEN_*` tokens instead
    // of one plain `COMMENT` token, so a naive "is this a `COMMENT` token"
    // token check misses it, and the `\code{x}` inside was flagged as an
    // unsupported raw Rd macro even though it is just comment text.
    let (model_output, sources) = model(
        r#"#' @name nested_roxygen_marker
#' @title Nested roxygen marker
#' @examples
#' #' see \code{x}
#' y <- 1
nested_roxygen_marker <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("nested_roxygen_marker".to_owned()))
        .unwrap_or_else(|| panic!("nested_roxygen_marker topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{nested_roxygen_marker}
    \alias{nested_roxygen_marker}
    \title{Nested roxygen marker}
    \usage{
    nested_roxygen_marker()
    }
    \description{
    Nested roxygen marker
    }
    \examples{
    #' see \\code\{x\}
    y <- 1
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_keeps_a_nested_roxygen_marker_inside_dontshow() {
    // The more serious shape of the same gap: a nested `#'` line's `}`
    // closed `\dontshow` early, with no diagnostic at all, leaking the code
    // meant to stay hidden (`z <- 2`) out into the ordinary example body --
    // exactly the "silently escaped Rd macro" failure class this module
    // exists to close, just triggered by a roxygen line rather than a raw
    // Rd macro. The snapshot below is what pins this: if `\dontshow`'s
    // brace closed early, `z <- 2` would show up outside it.
    let (model_output, sources) = model(
        r#"#' @name nested_roxygen_marker_in_dontshow
#' @title Nested roxygen marker in dontshow
#' @examples
#' \dontshow{
#' #' a } comment
#' z <- 2
#' }
#' after()
nested_roxygen_marker_in_dontshow <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("nested_roxygen_marker_in_dontshow".to_owned()))
        .unwrap_or_else(|| {
            panic!(
                "nested_roxygen_marker_in_dontshow topic: {:?}",
                output.diagnostics
            )
        });
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{nested_roxygen_marker_in_dontshow}
    \alias{nested_roxygen_marker_in_dontshow}
    \title{Nested roxygen marker in dontshow}
    \usage{
    nested_roxygen_marker_in_dontshow()
    }
    \description{
    Nested roxygen marker in dontshow
    }
    \examples{
    \dontshow{
    #' a \} comment
    z <- 2
    }
    after()
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn generated_document_diagnoses_an_unsupported_raw_macro_in_examples() {
    let (model_output, sources) = model(
        r#"#' @name unsupported_macro
#' @title Unsupported macro
#' @examples
#' before()
#' \link{foo}
#' after()
unsupported_macro <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(
        output.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::UnsupportedRawRdMacro
        }),
        "{:?}",
        output.diagnostics
    );
    // An error-severity diagnostic keeps the topic from being emitted at
    // all, matching every other error-severity diagnostic in this crate:
    // mini-roxygen must not write broken Rd for a topic it already knows is
    // wrong, rather than writing it and hoping `R CMD check` catches it.
    assert!(
        !output
            .files
            .contains_key(&TopicKey("unsupported_macro".to_owned())),
        "an unsupported raw Rd macro should keep the topic from being emitted"
    );
}

#[test]
fn generated_document_diagnoses_a_bracket_option_macro_in_examples() {
    // `\link[pkg]{foo}` was the gap the `{`-only check left open: `link`
    // followed immediately by `{` was recognized as macro-shaped, but
    // `link` followed by a bracket option before the `{` was not, so it
    // fell through as literal text and broke the same way `\dontrun` did
    // before this fix (bracket options are never valid R outside `\(`).
    let (model_output, sources) = model(
        r#"#' @name bracket_option_macro
#' @title Bracket option macro
#' @examples
#' before()
#' \link[pkg]{foo}
#' after()
bracket_option_macro <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(
        output.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::UnsupportedRawRdMacro
        }),
        "{:?}",
        output.diagnostics
    );
    assert!(
        !output
            .files
            .contains_key(&TopicKey("bracket_option_macro".to_owned())),
        "a bracket-option raw Rd macro should keep the topic from being emitted"
    );
}

#[test]
fn generated_document_diagnoses_a_braceless_macro_form_in_examples() {
    // `\R` has no brace group at all. Outside a string, R accepts a bare
    // backslash only as `\(`; anything else -- including a zero-argument
    // Rd macro name with nothing after it -- is not valid R and must be
    // diagnosed rather than passed through as literal text.
    let (model_output, sources) = model(
        r#"#' @name braceless_macro
#' @title Braceless macro
#' @examples
#' before()
#' \R
#' after()
braceless_macro <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(
        output.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::UnsupportedRawRdMacro
        }),
        "{:?}",
        output.diagnostics
    );
    assert!(
        !output
            .files
            .contains_key(&TopicKey("braceless_macro".to_owned())),
        "a braceless macro form should keep the topic from being emitted"
    );
}

#[test]
fn generated_document_diagnoses_an_unterminated_dontrun() {
    let (model_output, sources) = model(
        r#"#' @name unterminated_dontrun
#' @title Unterminated dontrun
#' @examples
#' before()
#' \dontrun{
#' never_closes()
unterminated_dontrun <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code == crate::diagnostic::DiagnosticCode::UnterminatedRawRdMacro
        })
        .unwrap_or_else(|| panic!("{:?}", output.diagnostics));
    // `\dontrun` is a recognized macro; the problem is specifically that its
    // brace never closes, so this gets its own code and message instead of
    // reusing `UnsupportedRawRdMacro` (whose doc comment says the macro name
    // itself is outside the supported set, which is not true here), and the
    // label points at just the introducer instead of the rest of the block.
    assert_eq!(diagnostic.message, "unterminated raw Rd macro");
    assert_eq!(
        sources.span_text(diagnostic.primary.span),
        Some("\\dontrun{")
    );
    assert!(
        !output
            .files
            .contains_key(&TopicKey("unterminated_dontrun".to_owned())),
        "an unterminated \\dontrun should keep the topic from being emitted"
    );
}

#[test]
fn generated_document_lowers_dontrun_inside_an_examples_if_body() {
    let (model_output, sources) = model(
        r#"#' @name conditional_dontrun
#' @title Conditional dontrun
#' @examplesIf interactive()
#' value <- 1
#' \dontrun{
#' slow_call()
#' }
conditional_dontrun <- function(value) value
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("conditional_dontrun".to_owned()))
        .unwrap_or_else(|| panic!("conditional_dontrun topic: {:?}", output.diagnostics));
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(generated.content, @r"
    % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
    % Please edit documentation in R/test.R
    \name{conditional_dontrun}
    \alias{conditional_dontrun}
    \title{Conditional dontrun}
    \usage{
    conditional_dontrun(value)
    }
    \description{
    Conditional dontrun
    }
    \examples{
    \dontshow{if (\{
    interactive()
    \}) withAutoprint(\{ # examplesIf}
    value <- 1
    \dontrun{
    slow_call()
    }
    \dontshow{\}) # examplesIf}
    }
    ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn ordinary_and_conditional_examples_are_singleton_duplicates() {
    let (model_output, _) = model(
        r#"#' @name merged
#' @title Merged examples
#' @examples
#' value <- 1
#' @examplesIf interactive()
#' value <- 2
merged <- function(value) value
"#,
    );
    let topic = model_output
        .package
        .topics
        .get(&TopicKey("merged".to_owned()))
        .expect("merged topic");
    assert!(matches!(
        topic.examples.as_ref().map(|value| &value.value),
        Some(crate::tags::ExamplesContent::Ordinary(value))
            if value.as_str() == "value <- 1"
    ));
    let duplicate = model_output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag)
        .expect("ordinary and conditional duplicate");
    assert_eq!(duplicate.secondary.len(), 1);
    assert_eq!(duplicate.primary.message, "second @examples contribution");
    assert_eq!(
        duplicate.secondary[0].message,
        "first @examples contribution"
    );
}

#[test]
fn repeated_examples_forms_are_source_aware_singleton_duplicates() {
    for source in [
        r#"#' @name ordinary_twice
#' @title Ordinary twice
#' @examples first <- 1
#' @examples first <- 1
ordinary_twice <- function() NULL
"#,
        r#"#' @name conditional_twice
#' @title Conditional twice
#' @examplesIf interactive()
#' first <- 1
#' @examplesIf interactive()
#' second <- 2
conditional_twice <- function() NULL
"#,
    ] {
        let (model_output, _) = model(source);
        let topic = model_output
            .package
            .topics
            .values()
            .next()
            .expect("examples topic");
        assert!(topic.examples.is_some());
        let duplicate = model_output
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag)
            .expect("repeated examples diagnostic");
        assert_eq!(duplicate.secondary.len(), 1);
        assert_eq!(duplicate.primary.message, "second @examples contribution");
        assert_eq!(
            duplicate.secondary[0].message,
            "first @examples contribution"
        );
        assert_ne!(duplicate.primary.span, duplicate.secondary[0].span);
    }
}

#[test]
fn merged_examples_are_topic_singleton_duplicates() {
    let (model_output, _) = model(
        r#"#' @name merged
#' @title Merged examples
#' @rdname merged
#' @examples
#' ordinary_one <- 1
#' @examplesIf interactive()
#' conditional_one <- 2
merged <- function() NULL

#' @rdname merged
#' @examplesIf requireNamespace("samplepkg", quietly = TRUE)
#' conditional_two <- 3
second <- function() NULL

#' @rdname merged
#' @examples
#' ordinary_two <- 4
third <- function() NULL
"#,
    );
    let topic = model_output
        .package
        .topics
        .get(&TopicKey("merged".to_owned()))
        .expect("merged topic");
    assert!(matches!(
        topic.examples.as_ref().map(|value| &value.value),
        Some(crate::tags::ExamplesContent::Ordinary(value))
            if value.as_str() == "ordinary_one <- 1"
    ));
    let duplicates = model_output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag)
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 3);
    for duplicate in duplicates {
        assert_eq!(duplicate.secondary.len(), 1);
        assert_eq!(duplicate.primary.message, "second @examples contribution");
        assert_eq!(
            duplicate.secondary[0].message,
            "first @examples contribution"
        );
    }
}

#[test]
fn malformed_examples_if_does_not_block_examples_inheritance() {
    let (model_output, tag_diagnostics, sources) = model_with_tag_diagnostics_and_sources(
        r#"#' @name donor
#' @title Donor
#' @examples
#' donor_value <- 1
donor <- function() NULL

#' @name recipient
#' @title Recipient
#' @examplesIf invalid(
#' @examplesIf interactive()
#' @inherit donor examples
recipient <- function() NULL
"#,
    );
    assert_eq!(
        tag_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == crate::diagnostic::DiagnosticCode::InvalidExamplesIfCondition
            })
            .count(),
        1
    );
    assert_eq!(
        tag_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == crate::diagnostic::DiagnosticCode::EmptyExamplesIfBody
            })
            .count(),
        1
    );

    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = output
        .files
        .get(&TopicKey("recipient".to_owned()))
        .expect("recipient.Rd");
    insta::assert_snapshot!(&generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in test.R
        \name{recipient}
        \alias{recipient}
        \title{Recipient}
        \usage{
        recipient()
        }
        \description{
        Recipient
        }
        \examples{
        donor_value <- 1
        }
        ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
    crate::rd_oracle::assert_r_examples_parse(&generated.content);
}

#[test]
fn inherited_examples_keep_one_donor_contribution() {
    let (model_output, sources) = model(
        r#"#' @name donor
#' @title Donor
#' @examplesIf interactive()
#' donor_value <- 1
donor <- function() NULL

#' @name recipient
#' @title Recipient
#' @inherit donor examples
recipient <- function() NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    let generated = output
        .files
        .get(&TopicKey("recipient".to_owned()))
        .expect("recipient.Rd");
    insta::assert_snapshot!(&generated.content, @r###"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{recipient}
        \alias{recipient}
        \title{Recipient}
        \usage{
        recipient()
        }
        \description{
        Recipient
        }
        \examples{
        \dontshow{if (\{
        interactive()
        \}) withAutoprint(\{ # examplesIf}
        donor_value <- 1
        \dontshow{\}) # examplesIf}
        }
        "###);
    crate::rd_oracle::assert_r_accepts(&generated.content);
    crate::rd_oracle::assert_r_examples_parse(&generated.content);
}

#[test]
fn ordinary_null_examples_are_literal_r_code() {
    let (model_output, sources) = model(
        r#"#' @name null_examples
#' @title Null examples
#' @examples NULL
null_examples <- function() NULL
"#,
    );
    let topic = model_output
        .package
        .topics
        .get(&TopicKey("null_examples".to_owned()))
        .expect("null_examples topic");
    assert!(topic.examples.is_some());
    assert!(matches!(
        &topic.examples.as_ref().expect("examples").value,
        crate::tags::ExamplesContent::Ordinary(value) if value.as_str() == "NULL"
    ));
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .get(&TopicKey("null_examples".to_owned()))
        .expect("null_examples.Rd");
    insta::assert_snapshot!(&generated.content, @r###"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{null_examples}
        \alias{null_examples}
        \title{Null examples}
        \usage{
        null_examples()
        }
        \description{
        Null examples
        }
        \examples{
        NULL
        }
        "###);
    crate::rd_oracle::assert_r_accepts(&generated.content);
    crate::rd_oracle::assert_r_examples_parse(&generated.content);
}

#[test]
fn inherited_conditional_examples_keep_their_wrapper() {
    let (model_output, sources) = model(
        r#"#' @name donor
#' @title Donor topic
#' @examplesIf interactive()
#' value <- 1
donor <- function(value) value

#' @name recipient
#' @title Recipient topic
#' @inherit donor examples
recipient <- function(value) value
"#,
    );
    let resolved_package = resolved(&model_output.package);
    let output = build_rd(&resolved_package, &sources);
    let generated = output
        .files
        .get(&TopicKey("recipient".to_owned()))
        .unwrap_or_else(|| panic!("recipient topic: {:?}", output.diagnostics));

    insta::assert_snapshot!(generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{recipient}
        \alias{recipient}
        \title{Recipient topic}
        \usage{
        recipient(value)
        }
        \description{
        Recipient topic
        }
        \examples{
        \dontshow{if (\{
        interactive()
        \}) withAutoprint(\{ # examplesIf}
        value <- 1
        \dontshow{\}) # examplesIf}
        }
        ");
}

#[test]
fn examples_inheritance_is_local_first_and_null_suppressible() {
    let (model_output, _) = model(
        r#"#' @name donor
#' @title Donor
#' @examples donor_value <- 1
donor <- function() NULL

#' @name local
#' @title Local
#' @examples local_value <- 1
#' @inherit donor examples
local <- function() NULL

#' @name suppressed
#' @title Suppressed
#' @inherit donor examples
#' @inherit NULL examples
suppressed <- function() NULL
"#,
    );
    let package = resolved(&model_output.package);
    let local = package
        .topics
        .get(&TopicKey("local".to_owned()))
        .expect("local topic");
    assert!(matches!(
        local.examples.as_ref().map(|content| &content.value),
        Some(InheritableContent::Examples(
            crate::tags::ExamplesContent::Ordinary(value)
        )) if value.as_str() == "local_value <- 1"
    ));
    let suppressed = package
        .topics
        .get(&TopicKey("suppressed".to_owned()))
        .expect("suppressed topic");
    assert!(suppressed.examples.is_none());
}

#[test]
fn examples_inheritance_uses_the_first_available_donor() {
    let (model_output, _) = model(
        r#"#' @name donor_one
#' @title Donor one
#' @examples donor_one_value <- 1
donor_one <- function() NULL

#' @name donor_two
#' @title Donor two
#' @examples donor_two_value <- 2
donor_two <- function() NULL

#' @name recipient
#' @title Recipient
#' @inherit donor_one examples
#' @inherit donor_two examples
recipient <- function() NULL
"#,
    );
    let package = resolved(&model_output.package);
    let recipient = package
        .topics
        .get(&TopicKey("recipient".to_owned()))
        .expect("recipient topic");
    assert!(matches!(
        recipient.examples.as_ref().map(|content| &content.value),
        Some(InheritableContent::Examples(
            crate::tags::ExamplesContent::Ordinary(value)
        )) if value.as_str() == "donor_one_value <- 1"
    ));
}

#[test]
fn generated_usage_has_only_the_spaced_wrapper_newlines() {
    let (model_output, sources) = model(
        r"#' @name generated
#' @title Generated
generated <- function(x) x
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let content = &output
        .files
        .values()
        .next()
        .expect("generated topic")
        .content;
    assert!(content.contains("\\usage{\ngenerated(x)\n}"));
    assert!(!content.contains("\\usage{\ngenerated(x)\n\n}"));
}

#[test]
fn generated_usage_uses_alias_formals_and_constructor_names() {
    let (model_output, sources) = model(
        r#"existing <- function(value = 1, flag = "x") existing

#' @name alias
#' @title Alias
alias <- existing

#' @name object
#' @title Object
object <- base::new.env(parent = emptyenv())
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(
        &output.files[&TopicKey("alias".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{alias}
        \alias{alias}
        \title{Alias}
        \usage{
        alias(value = 1, flag = "x")
        }
        \description{
        Alias
        }
        "#
    );
    insta::assert_snapshot!(
        &output.files[&TopicKey("object".to_owned())].content,
        @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{object}
        \alias{object}
        \title{Object}
        \usage{
        object
        }
        \description{
        Object
        }
        "
    );
}

#[test]
fn generated_usage_classifies_proven_local_s3_methods() {
    let (model_output, sources) = model(
        r#"generic <- function(x) UseMethod("generic")

#' @name methods
#' @title Methods
generic.default <- function(x) x

#' @name methods
generic.data.frame <- function(x) x

#' @name methods
generic.alpha.beta <- function(x) x

#' @name methods
generic.NULL <- function(x) x

#' @name methods
`generic."foo"` <- function(x) x

#' @name methods
ordinary.foo.bar <- function(x) x

#' @name null_value
#' @title Null value
null_value <- NULL
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(
        &output.files[&TopicKey("methods".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{methods}
        \alias{methods}
        \alias{generic.default}
        \alias{generic.data.frame}
        \alias{generic.alpha.beta}
        \alias{generic.NULL}
        \alias{generic."foo"}
        \alias{ordinary.foo.bar}
        \title{Methods}
        \usage{
        \method{generic}{default}(x)

        \method{generic}{data.frame}(x)

        \method{generic}{alpha.beta}(x)

        \method{generic}{`NULL`}(x)

        \method{generic}{`"foo"`}(x)

        ordinary.foo.bar(x)
        }
        \description{
        Methods
        }
        "#
    );

    let null_content = &output.files[&TopicKey("null_value".to_owned())].content;
    insta::assert_snapshot!(null_content, @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{null_value}
        \alias{null_value}
        \title{Null value}
        \description{
        Null value
        }
        "#);
}

#[test]
fn generated_usage_preserves_explicit_method_class_quoting() {
    let (model_output, sources) = model(
        r#"#' @name authored_method
#' @title Authored method
#' @method generic `NULL`
authored_method <- function(x) x
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(
        &output.files[&TopicKey("authored_method".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{authored_method}
        \alias{authored_method}
        \title{Authored method}
        \usage{
        \method{generic}{`NULL`}(x)
        }
        \description{
        Authored method
        }
        "#
    );
}

#[test]
fn generated_usage_wraps_long_multiline_and_special_calls() {
    let (model_output, sources) = model(
        r#"#' @name long_call
#' @title Long call
long_call <- function(first_argument, second_argument, third_argument, fourth_argument, fifth_argument) first_argument

#' @name multiline_call
#' @title Multiline call
multiline_call <- function(value = g(
  1, # keep this comment
  2
), raw = r"{line one
line two}") value

#' @name replacement_call
#' @title Replacement call
`replacement_call<-` <- function(first_argument, second_argument, third_argument, fourth_argument, value) value

#' @name method_call
#' @title Method call
#' @method generic class
generic.class <- function(first_argument, second_argument, third_argument, fourth_argument, fifth_argument) first_argument
"#,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);

    insta::assert_snapshot!(
        &output.files[&TopicKey("long_call".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{long_call}
        \alias{long_call}
        \title{Long call}
        \usage{
        long_call(
          first_argument,
          second_argument,
          third_argument,
          fourth_argument,
          fifth_argument
        )
        }
        \description{
        Long call
        }
        "#
    );
    insta::assert_snapshot!(
        &output.files[&TopicKey("multiline_call".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{multiline_call}
        \alias{multiline_call}
        \title{Multiline call}
        \usage{
        multiline_call(
          value = g(
            1, # keep this comment
            2
          ),
          raw = r"{line one
        line two}"
        )
        }
        \description{
        Multiline call
        }
        "#
    );
    insta::assert_snapshot!(
        &output.files[&TopicKey("replacement_call".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{replacement_call}
        \alias{replacement_call}
        \alias{replacement_call<-}
        \title{Replacement call}
        \usage{
        replacement_call(
          first_argument,
          second_argument,
          third_argument,
          fourth_argument
        ) <- value
        }
        \description{
        Replacement call
        }
        "#
    );
    insta::assert_snapshot!(
        &output.files[&TopicKey("method_call".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{method_call}
        \alias{method_call}
        \alias{generic.class}
        \title{Method call}
        \usage{
        \method{generic}{class}(
          first_argument,
          second_argument,
          third_argument,
          fourth_argument,
          fifth_argument
        )
        }
        \description{
        Method call
        }
        "#
    );
}

#[test]
fn generated_usage_classifies_proven_installed_s3_methods() {
    let (mut model_output, sources) = model(
        r#"#' @name external_method
#' @title External method
remote.alpha.beta <- function(x) x
"#,
    );
    classify_usage_methods(
        &mut model_output.package,
        &sources,
        &InstalledGenericProvider,
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(!output.diagnostics.has_errors(), "{:?}", output.diagnostics);
    insta::assert_snapshot!(
        &output.files[&TopicKey("external_method".to_owned())].content,
        @r#"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{external_method}
        \alias{external_method}
        \alias{remote.alpha.beta}
        \title{External method}
        \usage{
        \method{remote.alpha}{beta}(x)
        }
        \description{
        External method
        }
        "#
    );
}

#[test]
fn preserves_usage_contribution_order_and_renders_s3_methods_structurally() {
    let (model_output, sources) = model(
        r"#' @name many
#' @title Many
f <- function(x) x

#' @name many
`g` <- function(y) y
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let content = &output.files.values().next().expect("many topic").content;
    assert!(content.contains("\\usage{\nf(x)\n\n`g`(y)\n}"));
    // Two generated usages keep merge order, separated by one blank line
    // rather than by the first entry's trailing newline alone.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{many}
        \alias{many}
        \alias{f}
        \alias{g}
        \title{Many}
        \usage{
        f(x)

        `g`(y)
        }
        \description{
        Many
        }
        ");

    let (model_output, sources) = model(
        r"#' @name method
#' @title Method
#' @method a cls
`a(b)` <- function(x) x
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let content = &output.files.values().next().expect("method topic").content;
    // The method macro replaces a call head that itself contains the
    // parenthesis a textual split would have cut at.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{method}
        \alias{method}
        \alias{a(b)}
        \title{Method}
        \usage{
        \method{a}{cls}(x)
        }
        \description{
        Method
        }
        ");

    let (model_output, sources) = model(
        r"#' @name explicit
#' @title Explicit
#' @usage NULL
f <- function(x) x

#' @name explicit
#' @usage explicit(custom)
g <- function(y) y
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let content = &output
        .files
        .values()
        .next()
        .expect("explicit topic")
        .content;
    // `@usage NULL` suppresses only its own block's contribution, so the
    // other block's explicit usage is all that remains.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{explicit}
        \alias{explicit}
        \alias{f}
        \alias{g}
        \title{Explicit}
        \usage{
        explicit(custom)
        }
        \description{
        Explicit
        }
        ");
}

#[test]
fn an_s3_method_usage_segments_a_multi_line_default() {
    // This drives the production `Generated + method` branch. A test that
    // hands a tail to the segmenter itself would pass even while this
    // branch built one leaf and refused the topic.
    let (model_output, sources) = model(
        r"#' @name print.foo
#' @title Print foo
#' @method print foo
print.foo <- function(
  x = g(
    1,
    2
  )
) x
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    let generated = output
        .files
        .values()
        .next()
        .unwrap_or_else(|| panic!("method topic: {:?}", output.diagnostics));
    insta::assert_snapshot!(generated.content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/test.R
        \name{print.foo}
        \alias{print.foo}
        \title{Print foo}
        \usage{
        \method{print}{foo}(x = g(1, 2))
        }
        \description{
        Print foo
        }
        ");
    crate::rd_oracle::assert_r_accepts(&generated.content);
}

#[test]
fn reports_normalized_filename_collisions_without_overwriting() {
    let (model_output, sources) = model(
        r"#' @name a+b
#' @title First
`a+b` <- function() {}

#' @name a-plus-b
#' @title Second
`a-plus-b` <- function() {}
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert!(output.files.is_empty());
    assert!(
        output.diagnostics.iter().any(|diagnostic| diagnostic.code
            == crate::diagnostic::DiagnosticCode::ConflictingRdFileName)
    );
}

#[test]
fn a_name_that_normalizes_away_is_refused_rather_than_hidden() {
    // roxygen2 would write these to `man/.Rd`, a hidden file every such
    // topic overwrites in turn. There is no correct filename left to match,
    // so refusing is the useful answer.
    for name in ["\u{65e5}\u{672c}\u{8a9e}", "-"] {
        let (model_output, sources) = model(&format!(
            r#"#' @name {name}
#' @title Title
`{name}` <- function() {{}}
"#
        ));
        let output = build_rd(&resolved(&model_output.package), &sources);
        assert!(output.files.is_empty(), "{name} should produce no file");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code
                    == crate::diagnostic::DiagnosticCode::UnnameableRdFile),
            "{name} should report an unnameable Rd file"
        );
    }
}

#[test]
fn a_topic_that_fails_on_its_own_does_not_claim_a_filename() {
    // The untitled topic normalizes to the same file as the valid one. It
    // is not a claim on that name, so the valid topic must still be built
    // and the untitled topic must still report why it was dropped.
    let (model_output, sources) = model(
        r"#' @name a+b
a_plus_b <- function() {}

#' @name a-plus-b
#' @title Second
`a-plus-b` <- function() {}
",
    );
    let output = build_rd(&resolved(&model_output.package), &sources);
    assert_eq!(
        output.files.keys().collect::<Vec<_>>(),
        vec![&TopicKey("a-plus-b".to_owned())]
    );
    let codes = output
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&crate::diagnostic::DiagnosticCode::MissingTopicTitle));
    assert!(!codes.contains(&crate::diagnostic::DiagnosticCode::ConflictingRdFileName));
}

#[test]
fn package_metadata_warnings_are_emitted_before_missing_package_title() {
    let root = tempfile::tempdir().expect("temporary package root");
    fs::create_dir(root.path().join("R")).expect("R directory should be creatable");
    fs::write(
        root.path().join("DESCRIPTION"),
        "Package: example\nVersion: 0.1.0\nAuthors@R: foo(\"Broken\")\n",
    )
    .expect("DESCRIPTION should be writable");
    let source = "#' @keywords internal\n\"_PACKAGE\"\n";
    let source_path = root.path().join("R/package.R");
    fs::write(&source_path, source).expect("R source should be writable");
    let mut inputs = PackageInputs::from_package_root(root.path()).expect("inputs should load");
    let blocks = crate::model::test_support::blocks(&mut inputs.sources, "test.R", source);
    let model = build_package_model_with_metadata(&inputs.sources, blocks, &inputs.metadata);
    assert!(model.diagnostics.is_empty());
    let inheritance = resolved(&model.package);
    let output = build_rd(&inheritance, &inputs.sources);
    assert!(output.files.is_empty());
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingPackageDescription)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::PackageAuthorsParse)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingPackageTitle)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code
                == crate::diagnostic::DiagnosticCode::MissingTopicTitle)
            .count(),
        0
    );
    let sentinel_span = model
        .package
        .topics
        .values()
        .next()
        .and_then(|topic| topic.package_metadata_diagnostics.as_ref())
        .expect("package metadata diagnostic state")
        .anchor;
    assert!(output.diagnostics.iter().all(|diagnostic| {
        !matches!(
            diagnostic.code,
            crate::diagnostic::DiagnosticCode::MissingPackageDescription
                | crate::diagnostic::DiagnosticCode::PackageAuthorsParse
                | crate::diagnostic::DiagnosticCode::MissingPackageTitle
        ) || diagnostic.primary.span == sentinel_span
    }));
}

#[test]
fn header_uses_all_contributing_files_including_suppressed_blocks() {
    let (model_output, sources) = model_with_sources(&[
        (
            "R/first.R",
            r"#' @name shared
#' @title Shared
#' @usage NULL
f <- function() {}
",
        ),
        (
            "R/second.R",
            r"#' @name shared
#' @usage shared()
g <- function() {}
",
        ),
    ]);
    let topic = model_output
        .package
        .topics
        .get(&TopicKey("shared".to_owned()))
        .expect("shared topic");
    assert_eq!(topic.blocks.len(), 2);
    let output = build_rd(&resolved(&model_output.package), &sources);
    let content = &output.files.values().next().expect("shared output").content;
    // The first block contributes only a suppressed usage, and still has to
    // appear in the header as a file that edits this topic.
    insta::assert_snapshot!(content, @r"
        % Generated by mini-roxygen (roxygen2 compatible): do not edit by hand
        % Please edit documentation in R/first.R, R/second.R
        \name{shared}
        \alias{shared}
        \alias{f}
        \alias{g}
        \title{Shared}
        \usage{
        shared()
        }
        \description{
        Shared
        }
        ");
}
