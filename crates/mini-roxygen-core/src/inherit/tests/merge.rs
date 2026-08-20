use super::*;

struct RdParameterProvider {
    topic: InheritableTopic,
}

impl DocumentationProvider for RdParameterProvider {
    fn get_topic(
        &self,
        request: &TopicRequest,
    ) -> Result<Option<InheritableTopic>, DocumentationError> {
        let TopicRequest::External { package, topic } = request else {
            return Ok(None);
        };
        (package == "pkg" && topic.0 == "donor")
            .then(|| self.topic.clone())
            .map_or(Ok(None), |topic| Ok(Some(topic)))
    }
}

fn rd_parameter_donor(names: &[&str]) -> InheritableTopic {
    let label = names
        .iter()
        .enumerate()
        .flat_map(|(index, name)| {
            let separator = (index != 0).then(|| RdNode::Text(", ".into()));
            separator.into_iter().chain([RdNode::tagged(
                rd_ast::RdTag::Code,
                None,
                vec![RdNode::RCode((*name).into())],
            )])
        })
        .collect();
    InheritableTopic {
        identity: DocumentationIdentity::External {
            package: "pkg".into(),
            topic: "donor".into(),
        },
        params: vec![InheritableParamGroup {
            names: names
                .iter()
                .map(|name| crate::tags::ParamName((*name).into()))
                .collect(),
            label: InheritableParamLabel::Rd(label),
            description: ResolvedContent {
                value: InheritableContent::Rd(vec![RdNode::Text("Description".into())]),
                provenance: InheritanceTrace {
                    source: DocumentationOrigin::External {
                        package: "pkg".into(),
                        topic: "donor".into(),
                        component: crate::tags::InheritField::Params,
                    },
                    requests: Vec::new(),
                },
            },
        }],
        fields: InheritableFields::default(),
        sections: Vec::new(),
        requests: Vec::new(),
    }
}

fn resolve_external_params(source: &str, donor_names: &[&str]) -> ResolvedPackageModel {
    let input = model(source);
    let provider = RdParameterProvider {
        topic: rd_parameter_donor(donor_names),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::Strict,
        external_source: ExternalPolicySource::Explicit,
    };
    resolve_inheritance(&input.package, None, &NO_LINKS, &provider, &options).package
}

#[test]
fn rd_labels_fallback_for_dot_toggle_and_formal_reorder() {
    let toggled = resolve_external_params(
        r#"#' @name target
#' @inheritParams pkg::donor
target <- function(.x) {}
"#,
        &["x"],
    );
    let toggled = &toggled.topics[&crate::model::TopicKey("target".into())].params[0];
    assert_eq!(toggled.names, [crate::tags::ParamName(".x".into())]);
    assert_eq!(toggled.label, InheritableParamLabel::Generated);

    let reordered = resolve_external_params(
        r#"#' @name target
#' @inheritParams pkg::donor
target <- function(y, x) {}
"#,
        &["x", "y"],
    );
    let reordered = &reordered.topics[&crate::model::TopicKey("target".into())].params[0];
    assert_eq!(
        reordered.names,
        [
            crate::tags::ParamName("y".into()),
            crate::tags::ParamName("x".into())
        ]
    );
    assert_eq!(reordered.label, InheritableParamLabel::Generated);
}

#[test]
fn rd_labels_survive_selection_union_but_multi_name_groups_are_all_or_nothing() {
    let union = resolve_external_params(
        r#"#' @name target
#' @inheritParams pkg::donor x
#' @inheritParams pkg::donor y
target <- function(x, y) {}
"#,
        &["x", "y"],
    );
    let union = &union.topics[&crate::model::TopicKey("target".into())].params[0];
    assert_eq!(
        union.label,
        InheritableParamLabel::Rd(vec![
            RdNode::tagged(rd_ast::RdTag::Code, None, vec![RdNode::RCode("x".into())],),
            RdNode::Text(", ".into()),
            RdNode::tagged(rd_ast::RdTag::Code, None, vec![RdNode::RCode("y".into())],),
        ])
    );

    let partial = resolve_external_params(
        r#"#' @name target
#' @inheritParams pkg::donor x
target <- function(x, y) {}
"#,
        &["x", "y"],
    );
    assert!(
        partial.topics[&crate::model::TopicKey("target".into())]
            .params
            .is_empty()
    );
}

#[test]
fn unions_repeated_parameter_selections_before_copying_a_group() {
    let input = model(
        r#"#' @name donor
#' @param x X
#' @param y Y
donor <- function(x, y) {}

#' @name target
#' @inheritParams donor x
#' @inheritParams donor y
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
    let topic = &output.package.topics[&crate::model::TopicKey("target".into())];
    assert_eq!(topic.params.len(), 2);
    assert_eq!(topic.params[0].names[0].0, "x");
    assert_eq!(topic.params[1].names[0].0, "y");
    assert_eq!(topic.params[0].description.provenance.requests.len(), 1);
    assert_eq!(topic.params[1].description.provenance.requests.len(), 1);
    assert_ne!(
        topic.params[0].description.provenance.requests[0],
        topic.params[1].description.provenance.requests[0]
    );
}

#[test]
fn parameter_merge_matrix_keeps_local_names_exact_and_donor_matching_dot_toggled() {
    let cases = [
        (
            "local exact coverage leaves the toggled formal for the donor",
            r#"#' @name donor
#' @param x donor
donor <- function(x) {}

#' @name target
#' @param x local
#' @inheritParams donor
target <- function(x, .x) {}
"#,
            vec![vec!["x".to_owned()], vec![".x".to_owned()]],
            0,
        ),
        (
            "both toggled formals can be covered by one donor name",
            r#"#' @name donor
#' @param x donor
donor <- function(x) {}

#' @name target
#' @inheritParams donor
target <- function(x, .x) {}
"#,
            vec![vec!["x".to_owned(), ".x".to_owned()]],
            0,
        ),
        (
            "a partially matching multi-name group is rejected",
            r#"#' @name donor
#' @param x,y shared
donor <- function(x, y) {}

#' @name target
#' @inheritParams donor x
target <- function(x, y) {}
"#,
            Vec::<Vec<String>>::new(),
            2,
        ),
        (
            "ellipsis makes a filtered multi-name group ineligible",
            r#"#' @name donor
#' @param x,... shared
donor <- function(x, ...) {}

#' @name target
#' @inheritParams donor x
target <- function(x, ...) {}
"#,
            Vec::<Vec<String>>::new(),
            2,
        ),
        (
            "ellipsis remains inheritable for an unfiltered request",
            r#"#' @name donor
#' @param x,... shared
donor <- function(x, ...) {}

#' @name target
#' @inheritParams donor
target <- function(x, ...) {}
"#,
            vec![vec!["x".to_owned(), "...".to_owned()]],
            0,
        ),
        (
            "formal groups precede local non-formal groups",
            r#"#' @name donor
#' @param x X
#' @param y Y
donor <- function(x, y) {}

#' @name target
#' @param extra extra
#' @inheritParams donor
target <- function(y, x) {}
"#,
            vec![
                vec!["y".to_owned()],
                vec!["x".to_owned()],
                vec!["extra".to_owned()],
            ],
            0,
        ),
        (
            "undocumented formals remain as warnings",
            r#"#' @name donor
#' @param x X
donor <- function(x) {}

#' @name target
#' @inheritParams donor
target <- function(x, y) {}
"#,
            vec![vec!["x".to_owned()]],
            1,
        ),
    ];

    for (name, source, expected, missing_warnings) in cases {
        let input = model(source);
        let output = resolve_inheritance(
            &input.package,
            None,
            &NO_LINKS,
            &EmptyProvider,
            &InheritanceOptions::default(),
        );
        let topic = &output.package.topics[&crate::model::TopicKey("target".into())];
        assert_eq!(param_names(topic), expected, "case: {name}");
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingParam)
                .count(),
            missing_warnings,
            "case: {name}"
        );
        assert!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingParam)
                .all(|diagnostic| diagnostic.severity == crate::diagnostic::Severity::Warning),
            "case: {name}"
        );
    }
}

#[test]
fn unsupported_selector_drops_the_request_instead_of_becoming_unfiltered() {
    let (input, parse_diagnostics) = model_with_tag_diagnostics(
        r#"#' @name donor
#' @param x X
donor <- function(x) {}

#' @name target
#' @inheritParams donor 1:2
target <- function(x) {}
"#,
    );
    assert!(
        parse_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedSelection)
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
            .params
            .is_empty()
    );
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedSelection)
    );
}

#[test]
fn single_value_merge_uses_local_first_donor_and_never_concatenates() {
    let input = model(
        r#"#' @name first
#' @title first
first <- function() {}

#' @name second
#' @title second
second <- function() {}

#' @name local
#' @title local
#' @inherit first title
#' @inherit second title
local <- function() {}

#' @name filled
#' @inherit first title
#' @inherit second title
filled <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let title = |name: &str| {
        let topic = &output.package.topics[&crate::model::TopicKey(name.into())];
        let Some(ResolvedContent {
            value: InheritableContent::Markdown(value),
            ..
        }) = topic.title.as_ref()
        else {
            panic!("expected markdown title");
        };
        value.as_str().to_owned()
    };
    assert_eq!(title("local"), "local");
    assert_eq!(title("filled"), "first");
}

#[test]
fn named_sections_keep_local_titles_and_append_missing_titles_by_request_order() {
    let input = model(
        r#"#' @name first
#' @section A: first A
#' @section B: first B
first <- function() {}

#' @name second
#' @section B: second B
#' @section C: second C
second <- function() {}

#' @name target
#' @section A: local A
#' @inherit first sections
#' @inherit second sections
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
    let sections = &output.package.topics[&crate::model::TopicKey("target".into())].sections;
    let titles = sections
        .iter()
        .map(|section| match &section.title.value {
            InheritableContent::Markdown(value) => value.as_str().to_owned(),
            _ => panic!("expected markdown section title"),
        })
        .collect::<Vec<_>>();
    assert_eq!(titles, ["A", "B", "C"]);
    assert!(sections[0].body.provenance.requests.is_empty());
    assert_eq!(sections[1].body.provenance.requests.len(), 1);
    assert_eq!(sections[2].body.provenance.requests.len(), 1);
}

#[test]
fn partially_cancelled_requests_keep_remaining_fields_and_graph_edges() {
    let input = model(
        r#"#' @name zdonor
#' @title Donor title
#' @description Donor description
zdonor <- function() {}

#' @name atarget
#' @inherit zdonor title description
#' @inherit NULL title
atarget <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let target = &output.package.topics[&crate::model::TopicKey("atarget".into())];
    assert!(target.title.is_none());
    assert!(matches!(
        target.description.as_ref().map(|content| &content.value),
        Some(InheritableContent::Markdown(value)) if value.as_str() == "Donor description"
    ));

    let options = InheritanceOptions::default();
    let substitutions = crate::inline_r::InlineRSubstitutions::builtins()
        .expect("built-in substitutions should be valid");
    let usage = crate::inline_r::InlineRUsage::new();
    let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
    let mut resolver = super::resolver::Resolver {
        package: &input.package,
        current_package: None,
        links: &NO_LINKS,
        inline_r_session: &session,
        provider: &EmptyProvider,
        options: &options,
        normalized: std::collections::BTreeMap::new(),
        memo: std::collections::BTreeMap::new(),
        external_memo: std::collections::BTreeMap::new(),
        diagnostics: crate::diagnostic::Diagnostics::new(),
    };
    resolver.prepare();
    let request = &resolver.normalized[&crate::model::TopicKey("atarget".into())][0];
    assert_eq!(
        request.effective_fields,
        vec![crate::tags::InheritField::Description]
    );
    let graph = resolver.graph();
    assert_eq!(
        graph.dependency_order,
        vec![
            crate::model::TopicKey("zdonor".into()),
            crate::model::TopicKey("atarget".into())
        ]
    );
}
