use std::fs;

use super::{
    TopicKey, build_package_model, build_package_model_with_bindings,
    build_package_model_with_metadata,
};
use crate::diagnostic::DiagnosticCode;
use crate::model::test_support::{blocks, model, model_with_tag_diagnostics};
use crate::model::{FormalNames, ResolvedUsage};
use crate::package::{PackageInputs, PackageMetadata};
use crate::tags::NamespaceTag;

fn package_model_with_description(source: &str, description: &str) -> super::ModelOutput {
    let root = tempfile::tempdir().expect("temporary package root should exist");
    fs::create_dir(root.path().join("R")).expect("R directory should exist");
    fs::write(root.path().join("DESCRIPTION"), description)
        .expect("DESCRIPTION should be writable");
    let mut inputs = PackageInputs::from_package_root(root.path()).expect("package should load");
    let blocks = blocks(&mut inputs.sources, "test.R", source);
    build_package_model_with_metadata(&inputs.sources, blocks, &inputs.metadata)
}

fn package_description(authors: &str) -> String {
    format!(
        "Package: example\nTitle: Default title\nVersion: 0.1.0\nDescription: Default description.\nURL: https://example.org\nBugReports: https://example.org/issues\nAuthors@R: {authors}\n"
    )
}

#[test]
fn propagates_direct_function_formals_to_a_documented_alias() {
    let output = model(
        r#"target <- function(x, y = 1) NULL

#' Alias topic.
alias <- target
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("alias".into()))
        .expect("alias topic");
    let FormalNames::Known(formals) = &topic.formals[0].names else {
        panic!("expected direct alias formals");
    };
    assert_eq!(
        formals
            .iter()
            .map(|formal| formal.name.0.as_str())
            .collect::<Vec<_>>(),
        ["x", "y"]
    );
}

#[test]
fn unresolved_and_cyclic_aliases_do_not_guess_formals() {
    let output = model(
        r#"#' Unresolved alias.
unresolved <- missing

#' Forward alias.
forward <- later
later <- function(z) NULL

#' Cyclic alias A.
cycle_a <- cycle_b

#' Cyclic alias B.
cycle_b <- cycle_a
"#,
    );
    for name in ["unresolved", "forward", "cycle_a", "cycle_b"] {
        let topic = output
            .package
            .topics
            .get(&TopicKey(name.into()))
            .expect("alias topic");
        assert!(matches!(
            topic.formals[0].names,
            FormalNames::Unknown { .. }
        ));
    }
}

fn count_diagnostics(output: &super::ModelOutput, code: DiagnosticCode) -> usize {
    output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .count()
}

#[test]
fn package_block_is_registered_once_in_the_model() {
    let mut source_map = crate::source::SourceMap::new();
    let blocks = blocks(
        &mut source_map,
        "package.R",
        "#' @title Package topic\n\"_PACKAGE\"\n",
    );
    let metadata = PackageMetadata::new("example", None).expect("package metadata is valid");
    let output = build_package_model_with_metadata(&source_map, blocks, &metadata);
    let topic = output
        .package
        .topics
        .get(&TopicKey("example-package".to_owned()))
        .expect("package topic should be present");
    assert_eq!(topic.blocks.len(), 1);
    assert_eq!(topic.formals.len(), 1);
}

#[test]
fn package_block_honors_explicit_rdname_and_name_alias() {
    let mut source_map = crate::source::SourceMap::new();
    let blocks = blocks(
        &mut source_map,
        "package.R",
        "#' @title Package topic\n#' @rdname custom-package\n#' @name custom.alias\n\"_PACKAGE\"\n",
    );
    let metadata = PackageMetadata::new("example", None).expect("package metadata is valid");
    let output = build_package_model_with_metadata(&source_map, blocks, &metadata);
    let topic = output
        .package
        .topics
        .get(&TopicKey("custom-package".to_owned()))
        .expect("explicit package rdname should select the topic key");
    assert!(
        topic
            .aliases
            .iter()
            .any(|alias| alias.name.0 == "custom.alias")
    );
}

#[test]
fn package_default_alias_yields_to_an_ordinary_topic_with_the_package_name() {
    for source in [
        r#"#' @title Package topic
#' @keywords internal
"_PACKAGE"
#' @title Function topic
example <- function() NULL
"#,
        r#"#' @title Function topic
example <- function() NULL
#' @title Package topic
#' @keywords internal
"_PACKAGE"
"#,
    ] {
        let output = package_model_with_description(source, &package_description("NULL"));
        assert!(output.diagnostics.is_empty(), "source: {source}");

        let package = &output.package.topics[&TopicKey("example-package".into())];
        let ordinary = &output.package.topics[&TopicKey("example".into())];
        assert_eq!(
            package
                .aliases
                .iter()
                .map(|alias| alias.name.0.as_str())
                .collect::<Vec<_>>(),
            ["example-package"]
        );
        assert_eq!(
            ordinary
                .aliases
                .iter()
                .map(|alias| alias.name.0.as_str())
                .collect::<Vec<_>>(),
            ["example"]
        );
    }
}

#[test]
fn implicit_name_and_generated_usage_are_modelled() {
    let output = model(
        r#"#' Title
f <- function(x) x
"#,
    );
    let topic = output.package.topics.get(&TopicKey("f".into())).unwrap();
    assert_eq!(topic.name.0, "f");
    assert_eq!(topic.aliases[0].name.0, "f");
    assert_eq!(topic.title.as_ref().unwrap().value.as_str(), "Title");
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(usage.usage, ResolvedUsage::Generated(_))
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn aliases_generate_usage_from_proven_target_formals_and_defaults() {
    let output = model(
        r#"existing <- function(value = 1, flag = "x") existing

#' Alias topic
alias <- existing
"#,
    );
    let topic = &output.package.topics[&TopicKey("alias".into())];
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(
            &usage.usage,
            ResolvedUsage::Generated(value) if value.as_str() == "alias(value = 1, flag = \"x\")"
        )
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn known_non_function_constructor_objects_receive_name_only_usage() {
    let output = model(
        r#"#' Object topic
object <- base::new.env(parent = emptyenv())
"#,
    );
    let topic = &output.package.topics[&TopicKey("object".into())];
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(
            &usage.usage,
            ResolvedUsage::Generated(value) if value.as_str() == "object"
        )
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn simple_and_unknown_constructor_calls_do_not_guess_object_usage() {
    let output = model(
        r#"#' Simple object
simple <- new.env()

#' Unknown object
unknown <- unknown_constructor()
"#,
    );
    for name in ["simple", "unknown"] {
        let topic = &output.package.topics[&TopicKey(name.into())];
        assert!(matches!(
            topic.usages.as_slice(),
            [usage] if matches!(usage.usage, ResolvedUsage::Absent)
        ));
    }
    assert!(output.diagnostics.is_empty());
}

#[test]
fn local_constructor_shadowing_blocks_name_only_usage() {
    let output = model(
        r#"new.env <- function(...) function(...) NULL

#' Object topic
object <- new.env()
"#,
    );
    let topic = &output.package.topics[&TopicKey("object".into())];
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(usage.usage, ResolvedUsage::Absent)
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn qualified_base_constructor_is_known_without_local_lookup() {
    let output = model(
        r#"#' Object topic
object <- base::new.env()

#' Internal object topic
internal_object <- base:::new.env()
"#,
    );
    for (name, usage_text) in [("object", "object"), ("internal_object", "internal_object")] {
        let topic = &output.package.topics[&TopicKey(name.into())];
        assert!(matches!(
            topic.usages.as_slice(),
            [usage] if matches!(
                &usage.usage,
                ResolvedUsage::Generated(value) if value.as_str() == usage_text
            )
        ));
    }
    assert!(output.diagnostics.is_empty());
}

#[test]
fn explicit_name_keeps_implicit_name_as_alias() {
    let output = model(
        r#"#' @name topic
#' @aliases topic f
f <- function() f
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("topic".into()))
        .unwrap();
    assert_eq!(topic.name.0, "topic");
    assert_eq!(
        topic
            .aliases
            .iter()
            .map(|x| x.name.0.as_str())
            .collect::<Vec<_>>(),
        ["topic", "f"]
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn rdname_merges_files_and_order_controls_generated_usage() {
    let mut source_map = crate::source::SourceMap::new();
    let mut first = blocks(
        &mut source_map,
        "a.R",
        r#"#' @rdname shared
#' @order 2
a <- function(a) a
"#,
    );
    let mut second = blocks(
        &mut source_map,
        "b.R",
        r#"#' @rdname shared
#' @order 1
b <- function(b) b
"#,
    );
    first.append(&mut second);
    let output = build_package_model(&source_map, first);
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(topic.name.0, "b");
    let usages = topic
        .usages
        .iter()
        .filter_map(|contribution| match &contribution.usage {
            ResolvedUsage::Generated(value) => Some(value.as_str().trim()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(usages, ["b(b)", "a(a)"]);
    assert_eq!(
        topic
            .aliases
            .iter()
            .map(|alias| alias.name.0.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn rdname_order_defaults_last_and_ties_keep_source_order() {
    let mut source_map = crate::source::SourceMap::new();
    let mut first = blocks(
        &mut source_map,
        "a.R",
        r#"#' @rdname shared
#' @order 1
a <- function(a) a
"#,
    );
    let mut second = blocks(
        &mut source_map,
        "b.R",
        r#"#' @rdname shared
#' @order 1
b <- function(b) b
"#,
    );
    let mut third = blocks(
        &mut source_map,
        "c.R",
        r#"#' @rdname shared
c <- function(c) c
"#,
    );
    first.append(&mut second);
    first.append(&mut third);

    let output = build_package_model(&source_map, first);
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .expect("merged topic");
    assert_eq!(topic.name.0, "a");
    assert_eq!(
        topic
            .aliases
            .iter()
            .map(|alias| alias.name.0.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    let usages = topic
        .usages
        .iter()
        .filter_map(|contribution| match &contribution.usage {
            ResolvedUsage::Generated(value) => Some(value.as_str().trim()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(usages, ["a(a)", "b(b)", "c(c)"]);
    assert!(output.diagnostics.is_empty());
}

#[test]
fn propagates_s7_constructor_formals_to_a_documented_alias() {
    let output = model(
        r#"RenderOptions <- S7::new_class(
  "RenderOptions",
  constructor = function(..., compact = TRUE) NULL
)

#' Synthetic options.
#' @rdname RenderOptions
new_render_options <- RenderOptions
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("RenderOptions".into()))
        .expect("S7 alias topic");
    let FormalNames::Known(formals) = &topic.formals[0].names else {
        panic!("expected known S7 constructor formals");
    };
    assert_eq!(
        formals
            .iter()
            .map(|formal| formal.name.0.as_str())
            .collect::<Vec<_>>(),
        ["...", "compact"]
    );
    let usage = topic
        .usages
        .iter()
        .find_map(|contribution| match &contribution.usage {
            ResolvedUsage::Generated(value) => Some(value.as_str()),
            _ => None,
        })
        .expect("generated S7 alias usage");
    assert_eq!(usage, "new_render_options(..., compact = TRUE)");
    assert!(output.diagnostics.is_empty());
}

#[test]
fn does_not_propagate_s7_metadata_after_reassignment() {
    let output = model(
        r#"RenderOptions <- new_class(
  "RenderOptions",
  constructor = function(..., compact = TRUE) NULL
)
RenderOptions <- make_unknown()

#' Synthetic options.
#' @rdname RenderOptions
new_render_options <- RenderOptions
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("RenderOptions".into()))
        .expect("documented alias topic");
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(usage.usage, ResolvedUsage::Absent)
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn reports_source_aware_s7_refusal_for_documented_direct_definition() {
    let source = r#"#' Unsupported class.
Foo <- new_class(class_name, constructor = make_constructor())
"#;
    let output = model(source);
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedS7Constructor)
        .expect("documented S7 refusal diagnostic");
    let start = source.find("new_class").unwrap() as u32;
    let end = source.trim_end().len() as u32;
    assert_eq!(diagnostic.primary.span.range.start(), start);
    assert_eq!(diagnostic.primary.span.range.end(), end);
    assert_eq!(diagnostic.primary.span.file.index(), 0);
}

#[test]
fn reports_source_aware_s7_refusals_for_missing_and_computed_constructors() {
    for source in [
        r#"#' Missing constructor.
Foo <- new_class("Foo")
"#,
        r#"#' Computed constructor.
Foo <- new_class("Foo", constructor = make_constructor())
"#,
    ] {
        let output = model(source);
        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedS7Constructor)
            .expect("documented S7 refusal diagnostic");
        let start = source.find("new_class").unwrap() as u32;
        assert_eq!(diagnostic.primary.span.range.start(), start);
        assert_eq!(
            diagnostic.primary.span.range.end(),
            source.trim_end().len() as u32
        );
    }
}

#[test]
fn reports_alias_rhs_and_origin_for_crossing_s7_refusal() {
    let source = r#"Foo <- new_class(class_name, constructor = make_constructor())

#' Alias with unsupported class metadata.
#' @rdname alias
alias <- Foo
"#;
    let output = model(source);
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedS7Constructor)
        .expect("alias S7 refusal diagnostic");
    let alias_rhs = source.rfind("Foo").unwrap() as u32;
    assert_eq!(diagnostic.primary.span.range.start(), alias_rhs);
    assert_eq!(diagnostic.primary.span.range.end(), alias_rhs + 3);
    assert_eq!(diagnostic.primary.span.file.index(), 0);
    assert_eq!(diagnostic.secondary.len(), 1);
}

#[test]
fn refuses_cross_file_s7_alias_propagation_without_load_order_proof() {
    let mut source_map = crate::source::SourceMap::new();
    let first = blocks(
        &mut source_map,
        "a.R",
        r#"Foo <- new_class(
  "Foo",
  constructor = function(value = 1) NULL
)
"#,
    );
    let second = blocks(
        &mut source_map,
        "b.R",
        r#"#' Cross-file alias.
#' @rdname cross_file
alias <- Foo
"#,
    );
    let first_source = source_map
        .get(crate::source::FileId::new(0))
        .expect("first source");
    let first_bindings = crate::r_parse::build_object_index(
        crate::arity_adapter::parse(first_source, crate::source::FileId::new(0)),
        crate::source::FileId::new(0),
    )
    .bindings;
    let second_source = source_map
        .get(crate::source::FileId::new(1))
        .expect("second source");
    let second_bindings = crate::r_parse::build_object_index(
        crate::arity_adapter::parse(second_source, crate::source::FileId::new(1)),
        crate::source::FileId::new(1),
    )
    .bindings;
    let mut all_blocks = first;
    all_blocks.extend(second);
    let mut bindings = first_bindings;
    bindings.extend(second_bindings);
    let output = build_package_model_with_bindings(&source_map, all_blocks, bindings);
    let topic = output
        .package
        .topics
        .get(&TopicKey("cross_file".into()))
        .expect("cross-file alias topic");
    assert!(matches!(
        topic.usages.as_slice(),
        [usage] if matches!(usage.usage, ResolvedUsage::Absent)
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn method_declarations_stay_attached_to_usages_in_merge_order() {
    let output = model(
        r#"#' @rdname shared
#' @order 2
#' @method print foo
#' @usage f()
f <- function() f
#' @rdname shared
#' @order 1
#' @method format bar
#' @usage g()
g <- function() g
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(topic.usages.len(), 2);
    let methods = topic
        .usages
        .iter()
        .map(|contribution| {
            let method = contribution.method.as_ref().unwrap();
            (
                method.generic.value.as_str(),
                method.class.value.as_str(),
                match &contribution.usage {
                    ResolvedUsage::Explicit(value) => value.value.as_str(),
                    _ => panic!("expected explicit usage"),
                },
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(methods, [("format", "bar", "g()"), ("print", "foo", "f()")]);
    assert!(output.diagnostics.is_empty());
}

#[test]
fn method_without_usage_keeps_an_absent_contribution() {
    let output = model(
        r#"#' @name manual
#' @method print foo
NULL
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("manual".into()))
        .unwrap();
    assert_eq!(topic.usages.len(), 1);
    assert!(topic.usages[0].method.is_some());
    assert!(matches!(topic.usages[0].usage, ResolvedUsage::Absent));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn duplicate_method_identity_names_both_declarations() {
    let output = model(
        r#"#' @rdname shared
#' @method print foo
f <- function() f
#' @rdname shared
#' @method print foo
g <- function() g
"#,
    );
    let duplicate = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateMethod)
        .expect("duplicate method diagnostic");
    assert_eq!(duplicate.secondary.len(), 1);
    assert_ne!(duplicate.primary.span, duplicate.secondary[0].span);
    assert_eq!(
        output.package.topics[&TopicKey("shared".into())]
            .usages
            .len(),
        2
    );
}

#[test]
fn prose_slots_are_retained_once() {
    let output = model(
        r#"#' @title Function details
#' @seealso see
#' @references refs
#' @note note
#' @format format
#' @source source
#' @author author
f <- function() f
"#,
    );
    let topic = output.package.topics.get(&TopicKey("f".into())).unwrap();
    assert_eq!(topic.see_also.as_ref().unwrap().value.as_str(), "see");
    assert_eq!(topic.references.as_ref().unwrap().value.as_str(), "refs");
    assert_eq!(topic.note.as_ref().unwrap().value.as_str(), "note");
    assert_eq!(topic.format.as_ref().unwrap().value.as_str(), "format");
    assert_eq!(topic.source.as_ref().unwrap().value.as_str(), "source");
    assert_eq!(topic.author.as_ref().unwrap().value.as_str(), "author");
    assert!(output.diagnostics.is_empty());
}

#[test]
fn prose_slots_reject_a_second_block_contribution() {
    for tag in [
        "seealso",
        "references",
        "note",
        "format",
        "source",
        "author",
    ] {
        let output = model(&format!(
            r#"#' @rdname shared
#' @{tag} first
f <- function() f
#' @rdname shared
#' @{tag} second
g <- function() g
"#
        ));
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag
                })
                .count(),
            1,
            "expected one duplicate diagnostic for @{tag}"
        );
    }
}

#[test]
fn seealso_repeats_are_source_aware_topic_duplicates() {
    let output = model(
        r#"#' @title Shared seealso
#' @rdname shared
#' @seealso First contribution.
#' @seealso Second contribution.
f <- function() f
#' @rdname shared
#' @seealso Third contribution.
g <- function() g
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .expect("shared topic");
    assert_eq!(
        topic.see_also.as_ref().unwrap().value.as_str(),
        "First contribution."
    );
    let duplicates = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateTag)
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2);
    for duplicate in duplicates {
        assert_eq!(duplicate.secondary.len(), 1);
        assert_eq!(duplicate.primary.message, "second @seealso contribution");
        assert_eq!(
            duplicate.secondary[0].message,
            "first @seealso contribution"
        );
        assert_ne!(duplicate.primary.span, duplicate.secondary[0].span);
    }
}

#[test]
fn exact_seealso_repeat_is_not_silently_deduplicated() {
    let output = model(
        r#"#' @title Exact seealso
#' @seealso Same contribution.
#' @seealso Same contribution.
f <- function() f
"#,
    );
    let topic = &output.package.topics[&TopicKey("f".into())];
    assert_eq!(
        topic
            .see_also
            .as_ref()
            .expect("first seealso")
            .value
            .as_str(),
        "Same contribution."
    );
    let duplicate = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateTag)
        .expect("exact repeat diagnostic");
    assert_eq!(duplicate.primary.message, "second @seealso contribution");
    assert_eq!(duplicate.secondary.len(), 1);
    assert_ne!(duplicate.primary.span, duplicate.secondary[0].span);
}

#[test]
fn empty_seealso_contributions_do_not_create_or_replace_a_section() {
    let (output, tag_diagnostics) = model_with_tag_diagnostics(
        r#"#' @title Empty seealso
#' @seealso
f <- function() f
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("f".into()))
        .expect("function topic");
    assert!(topic.see_also.is_none());
    assert!(output.diagnostics.is_empty());
    assert_eq!(
        tag_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::TagParseError)
            .count(),
        1
    );
}

#[test]
fn empty_seealso_tags_do_not_consume_the_singleton_slot() {
    let cases = [
        r#"#' @title Empty before
#' @seealso
#' @seealso First contribution.
f <- function() f
"#,
        r#"#' @title Empty after
#' @seealso First contribution.
#' @seealso
f <- function() f
"#,
        r#"#' @title Empty before valid
#' @seealso
#' @seealso First contribution.
f <- function() f
"#,
    ];

    for source in cases {
        let (output, tag_diagnostics) = model_with_tag_diagnostics(source);
        assert_eq!(
            tag_diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::TagParseError)
                .count(),
            1,
            "expected one empty @seealso diagnostic for {source}"
        );
        let topic = output
            .package
            .topics
            .get(&TopicKey("f".into()))
            .expect("function topic");
        let value = topic.see_also.as_ref().expect("non-empty seealso");
        assert_eq!(value.value.as_str(), "First contribution.");
        assert!(output.diagnostics.is_empty());
    }
}

#[test]
fn malformed_examples_if_does_not_drop_a_valid_local_contribution() {
    let (output, tag_diagnostics) = model_with_tag_diagnostics(
        r#"#' @title ExamplesIf recovery
#' @examplesIf invalid(
#' @examplesIf interactive()
#' @examplesIf interactive() && TRUE
#' local_value <- 1
f <- function() NULL
"#,
    );
    assert_eq!(
        tag_diagnostics
            .iter()
            .filter(|diagnostic| { diagnostic.code == DiagnosticCode::InvalidExamplesIfCondition })
            .count(),
        1
    );
    assert_eq!(
        tag_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::EmptyExamplesIfBody)
            .count(),
        1
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("f".into()))
        .expect("function topic");
    assert!(topic.examples.is_some());
    assert!(output.diagnostics.is_empty());
}

#[test]
fn aliases_null_suppresses_only_that_blocks_default_aliases() {
    let output = model(
        r#"#' @name first
#' @rdname shared
#' @aliases NULL explicit
f <- function() f
#' @name second
#' @rdname shared
g <- function() g
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(
        topic
            .aliases
            .iter()
            .map(|alias| alias.name.0.as_str())
            .collect::<Vec<_>>(),
        ["explicit", "second", "g"]
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn data_objects_merge_null_format_before_or_after_the_hub() {
    for satellite_first in [false, true] {
        let (satellite_order, hub_order) = if satellite_first { (-1, 1) } else { (2, 1) };
        let output = model(&format!(
            r#"#' Band membership
#' @format Each is a tibble with two variables and three observations
#' @order {hub_order}
"band_members"
#' @rdname band_members
#' @format NULL
#' @order {satellite_order}
"band_instruments"
#' @rdname band_members
#' @format NULL
#' @order 3
"band_instruments2"
"#
        ));
        let topic = output
            .package
            .topics
            .get(&TopicKey("band_members".into()))
            .expect("band_members topic");
        assert_eq!(topic.aliases.len(), 3);
        let mut aliases = topic
            .aliases
            .iter()
            .map(|alias| alias.name.0.as_str())
            .collect::<Vec<_>>();
        aliases.sort_unstable();
        assert_eq!(
            aliases,
            ["band_instruments", "band_instruments2", "band_members"]
        );
        assert_eq!(
            topic.format.as_ref().expect("hub format").value.as_str(),
            "Each is a tibble with two variables and three observations"
        );
        assert!(output.diagnostics.is_empty());
    }
}

#[test]
fn data_topics_supply_defaults_and_preserve_explicit_usage() {
    for (description, expected_usage) in [
        ("Package: example\nLazyData: true\n", "dataset"),
        ("Package: example\nLazyData: TRUE\n", "dataset"),
        ("Package: example\nLazyData: yes\n", "dataset"),
        ("Package: example\nLazyData: false\n", "data(dataset)"),
        ("Package: example\n", "data(dataset)"),
    ] {
        let output = package_model_with_description(
            r#"#' Dataset
"dataset"
"#,
            description,
        );
        let topic = &output.package.topics[&TopicKey("dataset".into())];
        assert_eq!(topic.kind, super::RdTopicKind::Data);
        assert_eq!(topic.keywords, [crate::tags::Keyword("datasets".into())]);
        assert!(matches!(
            &topic.usages[0].usage,
            ResolvedUsage::Generated(value) if value.as_str() == expected_usage
        ));
    }

    let output = package_model_with_description(
        r#"#' Dataset
#' @usage custom_dataset()
#' @keywords custom
"dataset"
#' @rdname dataset
#' @keywords datasets other
"other_dataset"
"#,
        "Package: example\nLazyData: false\n",
    );
    let topic = &output.package.topics[&TopicKey("dataset".into())];
    assert_eq!(topic.kind, super::RdTopicKind::Data);
    assert_eq!(
        topic.keywords,
        [
            crate::tags::Keyword("custom".into()),
            crate::tags::Keyword("datasets".into()),
            crate::tags::Keyword("other".into()),
        ]
    );
    assert!(matches!(
        &topic.usages[0].usage,
        ResolvedUsage::Explicit(value) if value.value.as_str() == "custom_dataset()"
    ));
    assert!(matches!(
        &topic.usages[1].usage,
        ResolvedUsage::Generated(value) if value.as_str() == "data(other_dataset)"
    ));
    assert!(output.diagnostics.is_empty());

    let suppressed = package_model_with_description(
        r#"#' Dataset
#' @keywords NULL
"dataset"
"#,
        "Package: example\n",
    );
    assert!(
        suppressed.package.topics[&TopicKey("dataset".into())]
            .keywords
            .is_empty()
    );

    for (lazy_data, expected_usage) in [(false, "data(`dataset name`)"), (true, "`dataset name`")] {
        let output = package_model_with_description(
            r#"#' Dataset
"dataset name"
"#,
            &format!("Package: example\nLazyData: {lazy_data}\n"),
        );
        let topic = &output.package.topics[&TopicKey("dataset name".into())];
        assert!(matches!(
            &topic.usages[0].usage,
            ResolvedUsage::Generated(value) if value.as_str() == expected_usage
        ));
    }

    let output = package_model_with_description(
        r#"#' Dataset
r"(a`b)"
"#,
        "Package: example\nLazyData: true\n",
    );
    let topic = &output.package.topics[&TopicKey("a`b".into())];
    assert!(matches!(
        &topic.usages[0].usage,
        ResolvedUsage::Generated(value) if value.as_str() == "`a\\`b`"
    ));

    let output = package_model_with_description(
        r#"#' Dataset
r"(a\b)"
"#,
        "Package: example\nLazyData: false\n",
    );
    let topic = &output.package.topics[&TopicKey("a\\b".into())];
    assert!(matches!(
        &topic.usages[0].usage,
        ResolvedUsage::Generated(value) if value.as_str() == "data(`a\\\\b`)"
    ));
}

#[test]
fn package_and_data_contributions_report_one_order_independent_kind_conflict() {
    for source in [
        r#"#' @rdname shared
"_PACKAGE"
#' @rdname shared
"dataset"
"#,
        r#"#' @rdname shared
"dataset"
#' @rdname shared
"_PACKAGE"
"#,
    ] {
        let output = package_model_with_description(source, &package_description("NULL"));
        let diagnostics = output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::ConflictingTopicKind)
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "source: {source}");
        assert_eq!(diagnostics[0].secondary.len(), 1);
        let topic = &output.package.topics[&TopicKey("shared".into())];
        assert_eq!(topic.kind, super::RdTopicKind::Package);
        assert_eq!(
            topic.title.as_ref().unwrap().value.as_str(),
            "example: Default title"
        );
        assert!(topic.package_metadata_diagnostics.is_some());
    }
}

#[test]
fn kind_conflict_provenance_survives_suppressed_aliases() {
    let source = r#"#' @rdname shared
#' @aliases NULL
"_PACKAGE"
#' @rdname shared
#' @aliases NULL
"dataset"
"#;
    let output = package_model_with_description(source, &package_description("NULL"));
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::ConflictingTopicKind)
        .expect("mixed kind diagnostic");
    let package_start = source.find("\"_PACKAGE\"").expect("package sentinel") as u32;
    let data_start = source.find("\"dataset\"").expect("data name") as u32;
    assert!(
        [
            diagnostic.primary.span.range.start(),
            diagnostic.secondary[0].span.range.start()
        ]
        .contains(&package_start)
    );
    assert!(
        [
            diagnostic.primary.span.range.start(),
            diagnostic.secondary[0].span.range.start()
        ]
        .contains(&data_start)
    );
    assert!(
        output.package.topics[&TopicKey("shared".into())]
            .aliases
            .is_empty()
    );
}

#[test]
fn ordinary_and_data_contributions_remain_a_permitted_merge() {
    let output = package_model_with_description(
        r#"#' @rdname shared
#' @title Ordinary contribution
ordinary <- function() NULL
#' @rdname shared
"dataset"
"#,
        &package_description("NULL"),
    );
    assert_eq!(
        output.package.topics[&TopicKey("shared".into())].kind,
        super::RdTopicKind::Data
    );
    assert_eq!(
        count_diagnostics(&output, DiagnosticCode::ConflictingTopicKind),
        0
    );
}

#[test]
fn inherit_null_is_retained_as_a_suppression() {
    let output = model(
        r#"#' @inherit NULL
#' @inheritParams NULL
f <- function() f
"#,
    );
    let topic = &output.package.topics[&TopicKey("f".into())];
    assert!(matches!(
        &topic.inheritance[0],
        crate::model::InheritanceRequest::Inherit {
            target: crate::tags::InheritTarget::Suppress(_),
            ..
        }
    ));
    assert!(matches!(
        &topic.inheritance[1],
        crate::model::InheritanceRequest::InheritParams {
            target: crate::tags::InheritTarget::Suppress(_),
            ..
        }
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn topic_name_prefers_explicit_name_over_rdname_key() {
    let output = model(
        r#"#' @name object
#' @rdname shared
f <- function() f
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(topic.name.0, "object");
    assert_eq!(topic.aliases[0].name.0, "object");
    assert!(output.diagnostics.is_empty());
}

#[test]
fn no_rd_keeps_namespace_but_not_a_topic() {
    let output = model(
        r#"#' @noRd
#' @export
f <- function() f
"#,
    );
    assert!(output.package.topics.is_empty());
    assert_eq!(output.package.namespace.len(), 1);
    assert!(output.package.namespace[0].object.is_some());
    assert!(output.diagnostics.is_empty());
}

#[test]
fn export_only_function_keeps_namespace_but_not_a_topic() {
    let output = model(
        r#"#' @export
f <- function(x) x
"#,
    );
    assert!(output.package.topics.is_empty());
    assert!(matches!(
        output.package.namespace.as_slice(),
        [request] if matches!(request.tag, NamespaceTag::Export(_))
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn export_only_backtick_quoted_s3_method_keeps_namespace_but_not_a_topic() {
    let output = model(
        r#"#' @export
`$.fixture_data_frame` <- function(x, name) x
"#,
    );
    assert!(output.package.topics.is_empty());
    assert_eq!(
        output.package.namespace[0]
            .object
            .as_ref()
            .expect("implicit method name")
            .as_str(),
        "$.fixture_data_frame"
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn namespace_only_null_blocks_keep_requests_without_topic_identity_diagnostics() {
    let output = model(
        r#"#' @useDynLib fixture, .registration = TRUE
NULL
#' @import rlang
NULL
#' @importFrom utils head
NULL
"#,
    );
    assert!(output.package.topics.is_empty());
    assert_eq!(output.package.namespace.len(), 3);
    assert!(output.diagnostics.iter().all(
        |diagnostic| diagnostic.code != crate::diagnostic::DiagnosticCode::MissingTopicIdentity
    ));
    assert!(matches!(
        output.package.namespace[0].tag,
        NamespaceTag::UseDynLib(_)
    ));
    assert!(matches!(
        output.package.namespace[1].tag,
        NamespaceTag::Import(_)
    ));
    assert!(matches!(
        output.package.namespace[2].tag,
        NamespaceTag::ImportFrom(_)
    ));
}

#[test]
fn export_with_title_builds_both_topic_and_namespace_request() {
    let output = model(
        r#"#' @title Function title
#' @export
f <- function() f
"#,
    );
    assert!(output.package.topics.contains_key(&TopicKey("f".into())));
    assert!(matches!(
        output.package.namespace.as_slice(),
        [request] if matches!(request.tag, NamespaceTag::Export(_))
    ));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn inherit_params_alone_does_not_build_a_topic() {
    let output = model(
        r#"#' @inheritParams donor
f <- function(x) x
"#,
    );
    assert!(output.package.topics.is_empty());
    assert!(output.diagnostics.is_empty());
}

#[test]
fn inherit_alone_builds_a_topic() {
    let output = model(
        r#"#' @inherit donor
f <- function(x) x
"#,
    );
    assert!(output.package.topics.contains_key(&TopicKey("f".into())));
    assert!(output.diagnostics.is_empty());
}

#[test]
fn params_and_sections_keep_merge_then_source_order() {
    let mut source_map = crate::source::SourceMap::new();
    let mut first = blocks(
        &mut source_map,
        "a.R",
        r#"#' @rdname shared
#' @order 2
#' @param z later z
#' @param x later x
#' @section Z: later z
#' @section X: later x
f <- function(z, x) f
"#,
    );
    let mut second = blocks(
        &mut source_map,
        "b.R",
        r#"#' @rdname shared
#' @order 1
#' @param y first y
#' @param a first a
#' @section Y: first y
#' @section A: first a
g <- function(y, a) g
"#,
    );
    first.append(&mut second);
    let output = build_package_model(&source_map, first);
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(
        topic
            .params
            .iter()
            .map(|param| param.name.0.as_str())
            .collect::<Vec<_>>(),
        ["y", "a", "z", "x"]
    );
    assert_eq!(
        topic
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>(),
        ["Y", "A", "Z", "X"]
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn composed_intro_details_are_one_topic_slot() {
    let output = model(
        r#"#' Intro title.
#'
#' Intro description.
#'
#' Intro detail.
#' @details Explicit detail.
f <- function() f
"#,
    );
    let topic = output.package.topics.get(&TopicKey("f".into())).unwrap();
    assert_eq!(
        topic.details.as_ref().unwrap().value.as_str(),
        "Intro detail.\n\nExplicit detail."
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn aliases_retain_spans_and_conflicts_name_both_topics() {
    let output = model(
        r#"#' @title First
#' @aliases common
f <- function() f
#' @title Second
#' @aliases common
g <- function() g
"#,
    );
    let conflicts = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::ConflictingAlias)
        .collect::<Vec<_>>();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].secondary.len(), 1);
    assert_ne!(conflicts[0].primary.span, conflicts[0].secondary[0].span);
    let first = output.package.topics.get(&TopicKey("f".into())).unwrap();
    assert!(first.aliases.iter().any(|alias| alias.name.0 == "common"));
    assert!(
        first
            .aliases
            .iter()
            .all(|alias| alias.span.range.start() > 0)
    );
}

#[test]
fn no_rd_block_does_not_create_a_topic_but_another_block_can() {
    let output = model(
        r#"#' @noRd
#' @rdname shared
#' @title hidden
f <- function() f
#' @rdname shared
#' @title visible
g <- function() g
"#,
    );
    let topic = output
        .package
        .topics
        .get(&TopicKey("shared".into()))
        .unwrap();
    assert_eq!(topic.title.as_ref().unwrap().value.as_str(), "visible");
    assert_eq!(topic.usages.len(), 1);
    assert_eq!(
        topic
            .aliases
            .iter()
            .map(|alias| alias.name.0.as_str())
            .collect::<Vec<_>>(),
        ["g"]
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn package_companion_explicit_fields_are_order_independent() {
    for source in [
        r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @title Explicit title
#' @description Explicit description.
#' @seealso Explicit seealso
#' @author Explicit author
NULL
"#,
        r#"#' @name example-package
#' @title Explicit title
#' @description Explicit description.
#' @seealso Explicit seealso
#' @author Explicit author
NULL
#' @keywords internal
"_PACKAGE"
"#,
    ] {
        let output = package_model_with_description(
            source,
            &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
        );
        let topic = &output.package.topics[&TopicKey("example-package".into())];
        assert_eq!(
            topic.title.as_ref().unwrap().value.as_str(),
            "Explicit title"
        );
        assert_eq!(
            topic.description.as_ref().unwrap().value.as_str(),
            "Explicit description."
        );
        assert_eq!(
            topic.see_also.as_ref().unwrap().value.as_str(),
            "Explicit seealso"
        );
        assert_eq!(
            topic.author.as_ref().unwrap().value.as_str(),
            "Explicit author"
        );
        assert!(topic.package_see_also.is_none());
        assert!(topic.package_author.is_none());
        assert!(output.diagnostics.is_empty());
    }
}

#[test]
fn package_companion_suppression_is_ored_across_orders() {
    for source in [
        r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @title NULL
#' @description NULL
#' @seealso NULL
#' @author NULL
NULL
"#,
        r#"#' @name example-package
#' @title NULL
#' @description NULL
#' @seealso NULL
#' @author NULL
NULL
#' @keywords internal
"_PACKAGE"
"#,
    ] {
        let output =
            package_model_with_description(source, &package_description("foo(\"Broken\")"));
        let topic = &output.package.topics[&TopicKey("example-package".into())];
        assert!(topic.title.is_none());
        assert!(topic.description.is_none());
        assert!(topic.see_also.is_none());
        assert!(topic.author.is_none());
        assert!(topic.package_see_also.is_none());
        assert!(topic.package_author.is_none());
        assert!(topic.description_suppressed);
        assert!(output.diagnostics.is_empty());
    }
}

#[test]
fn package_explicit_values_survive_null_in_either_order() {
    for (tag, explicit, value) in [
        ("title", "Explicit title", "Explicit title"),
        (
            "description",
            "Explicit description.",
            "Explicit description.",
        ),
        ("seealso", "Explicit seealso", "Explicit seealso"),
        ("author", "Explicit author", "Explicit author"),
    ] {
        for (package_first, package_block, explicit_block, null_block) in [
            (
                true,
                "#' @keywords internal\n\"_PACKAGE\"",
                format!("#' @name example-package\n#' @{tag} {explicit}\nNULL"),
                format!("#' @name example-package\n#' @{tag} NULL\nNULL"),
            ),
            (
                false,
                "#' @keywords internal\n\"_PACKAGE\"",
                format!("#' @name example-package\n#' @{tag} NULL\nNULL"),
                format!("#' @name example-package\n#' @{tag} {explicit}\nNULL"),
            ),
        ] {
            let source = if package_first {
                format!("{package_block}\n{explicit_block}\n{null_block}\n")
            } else {
                format!("{null_block}\n{package_block}\n{explicit_block}\n")
            };
            let output = package_model_with_description(
                &source,
                &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
            );
            let topic = &output.package.topics[&TopicKey("example-package".into())];
            match tag {
                "title" => assert_eq!(topic.title.as_ref().unwrap().value.as_str(), value),
                "description" => {
                    assert_eq!(topic.description.as_ref().unwrap().value.as_str(), value)
                }
                "seealso" => {
                    assert_eq!(topic.see_also.as_ref().unwrap().value.as_str(), value);
                    assert!(topic.package_see_also.is_none());
                }
                "author" => {
                    assert_eq!(topic.author.as_ref().unwrap().value.as_str(), value);
                    assert!(topic.package_author.is_none());
                }
                _ => unreachable!(),
            }
            assert_eq!(count_diagnostics(&output, DiagnosticCode::DuplicateTag), 0);
        }
    }
}

#[test]
fn package_explicit_duplicate_spans_never_use_fallback_origin() {
    for source in [
        r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @title First explicit title
NULL
#' @name example-package
#' @title Second explicit title
NULL
"#,
        r#"#' @name example-package
#' @title Second explicit title
NULL
#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @title First explicit title
NULL
"#,
    ] {
        let output = package_model_with_description(
            source,
            &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
        );
        let duplicates = output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateTag)
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 1);
        let duplicate = duplicates[0];
        assert_eq!(duplicate.secondary.len(), 1);
        let primary_range = duplicate.primary.span.range;
        let secondary_range = duplicate.secondary[0].span.range;
        assert_ne!(primary_range, secondary_range);
        let primary_text = &source[primary_range.start() as usize..primary_range.end() as usize];
        let secondary_text =
            &source[secondary_range.start() as usize..secondary_range.end() as usize];
        assert!(primary_text.contains("title"));
        assert!(secondary_text.contains("title"));
        assert!(!primary_text.contains("_PACKAGE"));
        assert!(!secondary_text.contains("_PACKAGE"));
    }
}

#[test]
fn package_metadata_diagnostics_are_deferred_and_topic_wide() {
    let cases = [
        (
            "Title: Default title\n",
            r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @title Late title
NULL
"#,
            DiagnosticCode::MissingPackageTitle,
        ),
        (
            "Description: Default description.\n",
            r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @description Late description.
NULL
"#,
            DiagnosticCode::MissingPackageDescription,
        ),
        (
            "Authors@R: foo(\"Broken\")\n",
            r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @author Late author
NULL
"#,
            DiagnosticCode::PackageAuthorsParse,
        ),
        (
            "Authors@R: foo(\"Broken\")\n",
            r#"#' @keywords internal
"_PACKAGE"
#' @name example-package
#' @author NULL
NULL
"#,
            DiagnosticCode::PackageAuthorsParse,
        ),
    ];
    for (tail, source, code) in cases {
        let description = format!("Package: example\nVersion: 0.1.0\n{tail}");
        let output = package_model_with_description(source, &description);
        assert_eq!(count_diagnostics(&output, code), 0);
        let topic = output
            .package
            .topics
            .get(&TopicKey("example-package".into()))
            .expect("package topic");
        assert!(topic.package_metadata_diagnostics.is_some());
    }

    let output = package_model_with_description(
        r#"#' @keywords internal
"_PACKAGE"
"#,
        "Package: example\nVersion: 0.1.0\nAuthors@R: foo(\"Broken\")\n",
    );
    assert_eq!(output.diagnostics.len(), 0);
    let topic = output
        .package
        .topics
        .get(&TopicKey("example-package".into()))
        .expect("package topic");
    let pending = topic
        .package_metadata_diagnostics
        .as_ref()
        .expect("pending package metadata diagnostics");
    assert!(pending.missing_description);
    assert!(pending.authors_parse_error.is_some());
}

#[test]
fn redirected_package_key_merges_companion_in_either_order() {
    for (package_first, companion) in [
        (
            true,
            "#' @name custom-package\n#' @title Explicit title\n#' @description Explicit description.\nNULL",
        ),
        (
            false,
            "#' @name custom-package\n#' @title NULL\n#' @description NULL\nNULL",
        ),
    ] {
        let package_block = "#' @keywords internal\n#' @rdname custom-package\n\"_PACKAGE\"";
        let source = if package_first {
            format!("{package_block}\n{companion}\n")
        } else {
            format!("{companion}\n{package_block}\n")
        };
        let output = package_model_with_description(
            &source,
            &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
        );
        assert_eq!(output.package.topics.len(), 1);
        let topic = &output.package.topics[&TopicKey("custom-package".into())];
        assert_eq!(topic.blocks.len(), 2);
        if package_first {
            assert_eq!(
                topic.title.as_ref().unwrap().value.as_str(),
                "Explicit title"
            );
            assert_eq!(
                topic.description.as_ref().unwrap().value.as_str(),
                "Explicit description."
            );
        } else {
            assert!(topic.title.is_none());
            assert!(topic.description.is_none());
            assert!(topic.description_suppressed);
        }
    }
}

#[test]
fn multiple_package_topics_get_independent_fallbacks_and_alias_conflicts() {
    let same_key = package_model_with_description(
        r#"#' @keywords internal
"_PACKAGE"
#' @keywords internal
#' @title NULL
#' @seealso NULL
"_PACKAGE"
"#,
        &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
    );
    let same_topic = &same_key.package.topics[&TopicKey("example-package".into())];
    assert!(same_topic.title.is_none());
    assert!(same_topic.package_see_also.is_none());
    assert_eq!(same_topic.blocks.len(), 2);
    assert_eq!(
        count_diagnostics(&same_key, DiagnosticCode::MissingPackageTitle),
        0
    );

    let anchor_source = r#"#' @keywords internal
"_PACKAGE"
#' @keywords internal
"_PACKAGE"
"#;
    let anchored = package_model_with_description(
        anchor_source,
        "Package: example\nVersion: 0.1.0\nAuthors@R: foo(\"Broken\")\n",
    );
    assert!(anchored.diagnostics.is_empty());
    let pending = anchored
        .package
        .topics
        .values()
        .next()
        .and_then(|topic| topic.package_metadata_diagnostics.as_ref())
        .expect("pending package metadata diagnostics");
    assert!(pending.missing_description);
    assert!(pending.authors_parse_error.is_some());
    let anchor_range = pending.anchor.range;
    assert!(
        anchor_source[anchor_range.start() as usize..anchor_range.end() as usize]
            .contains("_PACKAGE")
    );

    let different_keys = package_model_with_description(
        r#"#' @keywords internal
#' @rdname first-package
"_PACKAGE"
#' @keywords internal
#' @rdname second-package
"_PACKAGE"
"#,
        &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
    );
    assert_eq!(different_keys.package.topics.len(), 2);
    assert!(
        different_keys
            .package
            .topics
            .values()
            .all(|topic| topic.kind == super::RdTopicKind::Package)
    );
    assert_eq!(
        count_diagnostics(&different_keys, DiagnosticCode::ConflictingAlias),
        1
    );
    assert!(
        different_keys
            .package
            .topics
            .values()
            .all(|topic| topic.title.is_some() && topic.description.is_some())
    );
}

#[test]
fn package_topic_preserves_ordinary_usage_and_aliases() {
    let output = package_model_with_description(
        r#"#' @keywords internal
#' @rdname f
"_PACKAGE"
#' @rdname f
#' @title Function title
#' @aliases function-alias
f <- function(x) x
"#,
        &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
    );
    let topic = &output.package.topics[&TopicKey("f".into())];
    assert_eq!(topic.kind, super::RdTopicKind::Package);
    assert_eq!(
        topic.title.as_ref().unwrap().value.as_str(),
        "Function title"
    );
    assert!(topic.aliases.iter().any(|alias| alias.name.0 == "f"));
    assert!(
        topic
            .aliases
            .iter()
            .any(|alias| alias.name.0 == "function-alias")
    );
    assert_eq!(
        topic
            .usages
            .iter()
            .filter(|usage| matches!(usage.usage, ResolvedUsage::Generated(_)))
            .count(),
        1
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn ordinary_suppression_is_not_copied_from_orphan_or_no_rd_blocks() {
    let output = package_model_with_description(
        r#"#' @noRd
#' @rdname hidden
#' @description NULL
hidden <- function() hidden
#' @rdname hidden
#' @title Visible title
visible <- function() visible
#' @name orphan-null
#' @title Orphan title
#' @description NULL
NULL
"#,
        &package_description("person(\"Default\", \"Author\", role = \"aut\")"),
    );
    let topic = &output.package.topics[&TopicKey("hidden".into())];
    assert_eq!(topic.kind, super::RdTopicKind::Ordinary);
    assert!(!topic.description_suppressed);
    assert_eq!(
        topic.title.as_ref().unwrap().value.as_str(),
        "Visible title"
    );
    assert!(topic.description.is_none());
    assert!(!output.package.topics[&TopicKey("orphan-null".into())].description_suppressed);
}
