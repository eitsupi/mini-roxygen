use super::*;

#[test]
fn default_external_policy_is_inferred_off() {
    let options = InheritanceOptions::default();
    assert_eq!(options.external, ExternalInheritancePolicy::Off);
    assert_eq!(
        options.external_source,
        ExternalPolicySource::NoConfiguredLibrary
    );
}

#[test]
fn off_never_consults_an_external_provider() {
    struct PanicProvider;
    impl DocumentationProvider for PanicProvider {
        fn get_topic(
            &self,
            _request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            panic!("external provider must not be consulted in Off mode");
        }
    }
    let input = model(
        r#"#' @name target
#' @inherit pkg::donor title
target <- function() {}
"#,
    );
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::Off,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &PanicProvider, &options);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::ExternalInheritanceDisabled)
    );
}

#[test]
fn inherit_section_rejects_ambiguous_external_titles() {
    let input = model(
        r#"#' @name target
#' @title Target
#' @section A: local body
#' @inheritSection pkg::donor A
#' @inheritSection pkg::donor **A**
target <- function() {}
"#,
    );
    let provider = RdSectionsProvider {
        sections: BTreeMap::from([(
            "donor".to_owned(),
            vec![
                vec![RdNode::Text("A".to_owned())],
                vec![RdNode::Text("A".to_owned())],
            ],
        )]),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::Strict,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &provider, &options);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == DiagnosticCode::AmbiguousInheritedSection })
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::AmbiguousInheritedSection)
            .count(),
        1
    );
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::DuplicateInheritanceRequest)
            .count(),
        1
    );
    assert!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .sections
            .len()
            == 1
    );
}

#[test]
fn inherit_section_copies_one_external_section_with_provenance() {
    let input = model(
        r#"#' @name target
#' @title Target
#' @inheritSection pkg::donor Details
target <- function() {}
"#,
    );
    let provider = RdSectionsProvider {
        sections: BTreeMap::from([(
            "donor".to_owned(),
            vec![vec![RdNode::Text("Details".to_owned())]],
        )]),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::Strict,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &provider, &options);
    let section = &output.package.topics[&crate::model::TopicKey("target".into())].sections[0];
    assert!(matches!(
        &section.body.value,
        InheritableContent::Rd(nodes) if nodes == &vec![RdNode::Text("Body".to_owned())]
    ));
    assert_eq!(section.body.provenance.requests.len(), 1);
    assert!(!output.diagnostics.has_errors());
}

#[test]
fn external_lookup_is_memoized_and_none_differs_from_failure() {
    let input = model(
        r#"#' @name target
#' @inherit pkg::donor title
#' @inherit pkg::donor description
target <- function() {}
"#,
    );
    let missing = CountingProvider {
        calls: Cell::new(0),
        result: Ok(None),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &missing, &options);
    assert_eq!(missing.calls.get(), 1);
    assert!(
        output
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message.contains("not found"))
    );

    let failed = CountingProvider {
        calls: Cell::new(0),
        result: Err(DocumentationError {
            kind: DocumentationErrorKind::TopicUnreadable,
            package: Some("pkg".into()),
            topic: Some("donor".into()),
            detail: "fixture failure".into(),
        }),
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &failed, &options);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("fixture failure"))
    );
}

#[test]
fn strict_external_failures_are_errors() {
    let input = model(
        r#"#' @name target
#' @inherit pkg::donor title
target <- function() {}
"#,
    );
    let provider = CountingProvider {
        calls: Cell::new(0),
        result: Ok(None),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::Strict,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &provider, &options);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == crate::diagnostic::Severity::Error)
    );
}

#[test]
fn fully_cancelled_requests_do_not_create_cycles_or_lookup_diagnostics() {
    let input = model(
        r#"#' @name a
#' @title A
#' @inherit b title
#' @inherit NULL title
a <- function() {}

#' @name b
#' @inherit a title
b <- function() {}
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
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::InheritCycle)
    );
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("b".into())]
            .title
            .as_ref()
            .expect("b inherits a title")
            .provenance
            .requests
            .len(),
        1
    );
}

#[test]
fn cancelled_missing_requests_are_omitted_before_donor_lookup() {
    let input = model(
        r#"#' @name target
#' @inherit missing title
#' @inherit pkg::missing title
#' @inherit NULL title
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
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnresolvedInherit)
    );
}

#[test]
fn external_href_section_titles_use_only_their_visible_argument() {
    struct HrefProvider;

    impl DocumentationProvider for HrefProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { .. } = request else {
                return Ok(None);
            };
            let mut topic = external_title("href");
            topic.sections[0].title.value = InheritableContent::Rd(vec![RdNode::tagged(
                rd_ast::RdTag::Href,
                None,
                vec![
                    RdNode::group(vec![RdNode::Text("https://example.org".into())]),
                    RdNode::group(vec![RdNode::Text("label".into())]),
                ],
            )]);
            Ok(Some(topic))
        }
    }

    let input = model(
        r#"#' @name target
#' @section label: local body
#' @inherit pkg::href sections
target <- function() {}
"#,
    );
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &HrefProvider, &options);
    let sections = &output.package.topics[&crate::model::TopicKey("target".into())].sections;
    assert_eq!(sections.len(), 1);
}

#[test]
fn external_alias_results_are_not_reused_for_a_different_request_key() {
    struct AliasProvider {
        requests: RefCell<Vec<String>>,
    }
    impl DocumentationProvider for AliasProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { topic, .. } = request else {
                return Ok(None);
            };
            self.requests.borrow_mut().push(topic.0.clone());
            if topic.0 == "alias" {
                Ok(Some(external_title("hidden")))
            } else {
                Ok(None)
            }
        }
    }
    let input = model(
        r#"#' @name target
#' @inherit pkg::alias title
#' @inherit pkg::hidden title
target <- function() {}
"#,
    );
    let provider = AliasProvider {
        requests: RefCell::new(Vec::new()),
    };
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &provider, &options);
    assert_eq!(provider.requests.borrow().as_slice(), ["alias", "hidden"]);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("pkg::hidden")
                && diagnostic.message.contains("not found"))
    );
}

#[test]
fn external_sections_use_their_semantic_titles_as_merge_keys() {
    struct SectionProvider;
    impl DocumentationProvider for SectionProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { topic, .. } = request else {
                return Ok(None);
            };
            Ok(Some(external_title(&topic.0)))
        }
    }
    let input = model(
        r#"#' @name target
#' @inherit pkg::first sections
#' @inherit pkg::second sections
target <- function() {}
"#,
    );
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(&input.package, None, &NO_LINKS, &SectionProvider, &options);
    let sections = &output.package.topics[&crate::model::TopicKey("target".into())].sections;
    assert_eq!(sections.len(), 2);
    let titles = sections
        .iter()
        .map(|section| match &section.title.value {
            InheritableContent::Rd(nodes) => match nodes.as_slice() {
                [RdNode::Text(title)] => title.as_str(),
                _ => panic!("expected external text title"),
            },
            _ => panic!("expected external Rd title"),
        })
        .collect::<Vec<_>>();
    assert_eq!(titles, ["first", "second"]);
}
