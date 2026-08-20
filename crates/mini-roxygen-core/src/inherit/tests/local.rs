use super::*;

#[test]
fn resolves_a_local_inheritance_chain_and_keeps_the_input_model_unchanged() {
    let input = model(
        r#"#' @name c
#' @title C
#' @param x from C
c <- function(x) {}

#' @name b
#' @inherit c
b <- function(x) {}

#' @name a
#' @inherit b
a <- function(x) {}
"#,
    );
    let before = input.package.clone();
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(input.package, before);
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("a".into())]
            .title
            .as_ref()
            .expect("title")
            .provenance
            .requests
            .len(),
        2
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("a".into())].params[0].names[0].0,
        "x"
    );
    assert!(!output.diagnostics.has_errors());
}

#[test]
fn resolves_an_alias_and_current_package_qualification_locally() {
    let input = model(
        r#"#' @name donor
#' @aliases friendly
#' @title Donor
donor <- function() {}

#' @name target
#' @inherit friendly
target <- function() {}

#' @name qualified
#' @inherit self::donor
qualified <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        Some("self"),
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(output.package.topics.len(), 3);
    assert!(matches!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .title
            .as_ref()
            .expect("title")
            .provenance
            .source,
        super::DocumentationOrigin::Local(_)
    ));
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
    );
}

#[test]
fn resolves_local_inheritance_by_rdname_key_when_display_name_differs() {
    let input = model(
        r#"#' @name donor_display
#' @rdname donor-file
#' @title Donor
donor <- function() {}

#' @name target
#' @inherit donor-file
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(matches!(
        &output.package.topics[&crate::model::TopicKey("target".into())]
            .title
            .as_ref()
            .expect("inherited title")
            .value,
        super::InheritableContent::Markdown(value) if value.as_str() == "Donor"
    ));
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
    );
}

#[test]
fn recovers_each_cycle_member_with_local_content_only() {
    let input = model(
        r#"#' @name a
#' @title A
#' @inherit b
a <- function() {}

#' @name b
#' @title B
#' @inherit a
b <- function() {}

#' @name c
#' @inherit a
c <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::InheritCycle)
            .count(),
        1
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("a".into())]
            .title
            .as_ref()
            .expect("title")
            .provenance
            .requests
            .len(),
        0
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("c".into())]
            .title
            .as_ref()
            .expect("title")
            .provenance
            .requests
            .len(),
        1
    );
}

#[test]
fn reports_invalid_selection_and_skips_that_request() {
    let input = model(
        r#"#' @name donor
#' @title Donor
#' @param x X
#' @param y Y
donor <- function(x, y) {}

#' @name target
#' @inheritParams donor missing
target <- function(x, y) {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InvalidSelection)
    );
    assert!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .params
            .is_empty()
    );
}

#[test]
fn missing_targets_report_not_found_instead_of_ambiguity() {
    let input = model(
        r#"#' @name target
#' @inherit typo title
target <- function() {}

#' @name qualified_target
#' @inherit self::typo title
qualified_target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        Some("self"),
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let diagnostics = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message.contains("not found"))
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("ambiguous"))
    );
}

#[test]
fn inherit_section_is_local_first_and_repeated_requests_do_not_duplicate() {
    let input = model(
        r#"#' @name donor
#' @title Donor
#' @section Details: donor body
donor <- function() {}

#' @name target
#' @title Target
#' @section Details: local body
#' @inheritSection donor Details
#' @inheritSection donor Details
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert_eq!(target.sections.len(), 1);
    assert!(matches!(
        &target.sections[0].body.value,
        super::InheritableContent::Markdown(value) if value.as_str().trim() == "local body"
    ));
    assert!(!output.diagnostics.has_errors());
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn duplicate_inheritance_requests_reach_the_resolver_only_once() {
    let source = r#"#' @name donor
#' @title Donor
donor <- function() NULL

#' @name target
#' @inherit donor title
#' @inherit donor title
target <- function() NULL
"#;
    let input = model(source);
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let title = output.package.topics[&crate::model::TopicKey("target".into())]
        .title
        .as_ref()
        .expect("inherited title");
    assert_eq!(title.provenance.requests.len(), 1);
    let duplicates = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].severity, crate::diagnostic::Severity::Warning);
    assert_eq!(
        duplicates[0].primary.message,
        "later identical inheritance request"
    );
    assert_eq!(
        duplicates[0].secondary[0].message,
        "first identical inheritance request"
    );
    let first_start = source.find("@inherit donor title").expect("first request");
    let later_start = source.rfind("@inherit donor title").expect("later request");
    assert_eq!(
        duplicates[0].secondary[0].span.range.start() as usize,
        first_start
    );
    assert_eq!(
        duplicates[0].primary.span.range.start() as usize,
        later_start
    );
}

#[test]
fn duplicate_inherit_params_requests_are_deduplicated() {
    let input = model(
        r#"#' @name donor
#' @param x X
donor <- function(x) NULL

#' @name target
#' @inheritParams donor
#' @inheritParams donor
target <- function(x) NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert_eq!(target.params.len(), 1);
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn selector_order_difference_is_not_deduplicated() {
    let input = model(
        r#"#' @name donor
#' @param x X
#' @param y Y
donor <- function(x, y) NULL

#' @name target
#' @inheritParams donor x y
#' @inheritParams donor y x
target <- function(x, y) NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert_eq!(target.params.len(), 2);
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        0
    );
}

#[test]
fn alias_and_current_package_qualification_share_local_target_identity() {
    let input = model(
        r#"#' @name donor
#' @aliases friendly
#' @title Donor
donor <- function() NULL

#' @name target
#' @inherit friendly title
#' @inherit self::donor title
target <- function() NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        Some("self"),
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert_eq!(target.title.as_ref().unwrap().provenance.requests.len(), 1);
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn implicit_and_explicit_all_fields_share_request_identity() {
    let input = model(
        r#"#' @name donor
#' @title Donor
donor <- function() NULL

#' @name target
#' @inherit donor
#' @inherit donor params return title description details seealso sections references examples author source note format
target <- function() NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn duplicate_requests_from_rdname_blocks_warn_once() {
    let input = model(
        r#"#' @rdname shared
#' @inherit donor title
first <- function() NULL

#' @rdname shared
#' @inherit donor title
second <- function() NULL

#' @name donor
#' @title Donor
donor <- function() NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn different_donors_keep_fallback_order() {
    let input = model(
        r#"#' @name first
#' @title First
first <- function() NULL

#' @name second
#' @title Second
second <- function() NULL

#' @name target
#' @inherit first title
#' @inherit second title
target <- function() NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let title = output.package.topics[&crate::model::TopicKey("target".into())]
        .title
        .as_ref()
        .expect("inherited title");
    assert!(matches!(
        &title.value,
        InheritableContent::Markdown(value) if value.as_str() == "First"
    ));
}

#[test]
fn null_suppression_coexists_with_duplicate_requests_without_changing_scope() {
    let input = model(
        r#"#' @name donor
#' @title Donor
donor <- function() NULL

#' @name target
#' @inherit donor title
#' @inherit NULL title
#' @inherit donor title
target <- function() NULL
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert!(target.title.is_none());
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
}

#[test]
fn inherit_section_reports_missing_donor_and_section_without_guessing() {
    let input = model(
        r#"#' @name donor
#' @title Donor
donor <- function() {}

#' @name target
#' @title Target
#' @inheritSection absent Details
#' @inheritSection donor Missing
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::UnresolvedInherit
            && diagnostic.message.contains("not found")
    }));
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::MissingInheritedSection)
    );
}

#[test]
fn semantically_equal_inherit_section_titles_are_deduplicated() {
    let source = r#"#' @name donor
#' @title Donor
donor <- function() {}

#' @name target
#' @inheritSection donor A
#' @inheritSection donor **A**
target <- function() {}
"#;
    let input = model(source);
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let duplicates = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
        .collect::<Vec<_>>();
    let missing = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingInheritedSection)
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(missing.len(), 1);
    let first_start = source
        .find("@inheritSection donor A")
        .expect("first request");
    let later_start = source
        .rfind("@inheritSection donor **A**")
        .expect("later request");
    assert_eq!(
        duplicates[0].secondary[0].span.range.start() as usize,
        first_start
    );
    assert_eq!(
        duplicates[0].primary.span.range.start() as usize,
        later_start
    );
}

#[test]
fn different_inherit_section_titles_remain_distinct() {
    let input = model(
        r#"#' @name donor
#' @title Donor
donor <- function() {}

#' @name target
#' @inheritSection donor A
#' @inheritSection donor B
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        0
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingInheritedSection)
            .count(),
        2
    );
}

#[test]
fn local_section_does_not_hide_a_missing_donor_section() {
    let input = model(
        r#"#' @name donor
#' @title Donor
donor <- function() {}

#' @name target
#' @title Target
#' @section Missing: local body
#' @inheritSection donor Missing
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::MissingInheritedSection)
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .sections
            .len(),
        1
    );
}

#[test]
fn inherit_section_participates_in_cycle_detection() {
    let input = model(
        r#"#' @name first
#' @title First
#' @inheritSection second Details
#' @section Details: first body
first <- function() {}

#' @name second
#' @title Second
#' @inheritSection first Details
#' @section Details: second body
second <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InheritCycle)
    );
}

#[test]
fn null_inheritance_suppresses_fields_and_params_without_creating_graph_edges() {
    let input = model(
        r#"#' @name donor
#' @title Donor
#' @param x X
donor <- function(x) {}

#' @name target
#' @inherit donor title
#' @inherit NULL title
#' @inheritParams donor
#' @inheritParams NULL
target <- function(x) {}

#' @name NULL
#' @title Sentinel
#' @inherit target title
NULL <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert!(target.title.is_none());
    assert!(target.params.is_empty());
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InheritCycle)
    );
}

#[test]
fn ambiguous_aliases_are_unresolvable_instead_of_first_wins() {
    let input = model(
        r#"#' @name first
#' @aliases common
#' @title First
first <- function() {}

#' @name second
#' @aliases common
#' @title Second
second <- function() {}

#' @name target
#' @inherit common title
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    assert!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .title
            .is_none()
    );
    let diagnostic = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
        .expect("ambiguous alias diagnostic");
    assert!(diagnostic.message.contains("ambiguous"));
}
