use super::*;

#[test]
fn local_semantic_section_conflicts_report_both_source_spans_and_keep_first() {
    let input = model(
        r#"#' @name target
#' @section A: first body
#' @section **A**: later body
target <- function() {}
"#,
    );
    assert!(input.diagnostics.is_empty());

    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let diagnostics = output
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::ConflictingSectionTitle)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.primary.message, "conflicting section title");
    assert_eq!(diagnostic.secondary.len(), 1);
    assert_eq!(
        diagnostic.secondary[0].message,
        "first section with this title"
    );

    let sections = &output.package.topics[&crate::model::TopicKey("target".into())].sections;
    assert_eq!(sections.len(), 1);
    assert!(matches!(
        &sections[0].title.value,
        InheritableContent::Markdown(value) if value.as_str() == "A"
    ));
}

#[test]
fn markdown_titles_with_equal_semantic_text_share_one_section_key() {
    let input = model(
        r#"#' @name donor
#' @section A: donor body
donor <- function() {}

#' @name target
#' @section **A**: local body
#' @inherit donor sections
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
    assert_eq!(sections.len(), 1);
    assert!(matches!(
        &sections[0].title.value,
        InheritableContent::Markdown(value) if value.as_str() == "**A**"
    ));
    assert!(matches!(
        &sections[0].body.value,
        InheritableContent::Markdown(value) if value.as_str().trim() == "local body"
    ));
}

#[test]
fn substituted_section_titles_use_the_same_identity_as_final_rendering() {
    let substitutions =
        crate::inline_r::InlineRSubstitutions::builtins().expect("built-ins should validate");
    let usage = crate::inline_r::InlineRUsage::new();
    let session = crate::inline_r::InlineRSession::new(&substitutions, &usage);
    let title =
        crate::markdown_conversion::test_support::value("r lifecycle::badge(\"experimental\")");
    let key =
        crate::markdown_conversion::markdown_section_key(&title, None, &NO_LINKS, Some(&session));
    let links = crate::rd::LinkAdapter { links: &NO_LINKS };
    let context = crate::markdown_conversion::MarkdownContext {
        current_package: None,
        links: &links,
        inline_r_session: Some(&session),
    };
    let rendered = crate::markdown_conversion::convert_markdown(&title, &context);
    assert_eq!(
        key,
        crate::markdown_conversion::section_key::SectionTitleKey::from_rd(&rendered.fragment.nodes)
    );
}

#[test]
fn parseable_markdown_code_span_titles_merge_with_plain_section_titles() {
    let input = model(
        r#"#' @name donor
#' @section A: donor body
donor <- function() {}

#' @name target
#' @section `A`: local body
#' @inherit donor sections
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
        output.package.topics[&crate::model::TopicKey("target".into())]
            .sections
            .len(),
        1
    );
}

#[test]
fn non_parseable_markdown_code_span_titles_merge_with_plain_section_titles() {
    let input = model(
        r#"#' @name donor
#' @section not valid R %: donor body
donor <- function() {}

#' @name target
#' @section `not valid R %`: local body
#' @inherit donor sections
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
        output.package.topics[&crate::model::TopicKey("target".into())]
            .sections
            .len(),
        1
    );
}

#[test]
fn markdown_url_labels_merge_with_plain_section_titles() {
    let input = model(
        r#"#' @name donor
#' @section [label](https://example.org): donor body
donor <- function() {}

#' @name target
#' @section label: local body
#' @inherit donor sections
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
    assert_eq!(sections.len(), 1);
    assert!(matches!(
        &sections[0].body.value,
        InheritableContent::Markdown(value) if value.as_str().trim() == "local body"
    ));
}

#[test]
fn current_package_help_links_use_their_rendered_labels_as_section_keys() {
    let input = model(
        r#"#' @name donor
#' @section [pkg::obj]: donor body
donor <- function() {}

#' @name target
#' @section obj: local body
#' @inherit donor sections
target <- function() {}
"#,
    );
    let output = resolve_inheritance(
        &input.package,
        Some("pkg"),
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let sections = &output.package.topics[&crate::model::TopicKey("target".into())].sections;
    assert_eq!(sections.len(), 1);
}

#[test]
fn differently_rendered_section_titles_remain_separate() {
    let input = model(
        r#"#' @name donor
#' @section B: donor body
donor <- function() {}

#' @name target
#' @section A: local body
#' @inherit donor sections
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
    assert_eq!(sections.len(), 2);
}

#[test]
fn conditional_ifelse_titles_do_not_collide_with_plain_text() {
    let title = RdNode::tagged(
        rd_ast::RdTag::IfElse,
        None,
        vec![
            RdNode::group(vec![RdNode::Text("html".into())]),
            RdNode::group(vec![RdNode::Text("A".into())]),
            RdNode::group(vec![RdNode::Text("B".into())]),
        ],
    );
    assert_eq!(
        external_section_count(
            r#"#' @section AB: local body
#' @inherit pkg::conditional sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("conditional"), vec![vec![title]])]),
        ),
        2
    );
}

#[test]
fn conditional_if_titles_do_not_collide_with_plain_text() {
    let title = RdNode::tagged(
        rd_ast::RdTag::If,
        None,
        vec![
            RdNode::group(vec![RdNode::Text("html".into())]),
            RdNode::group(vec![RdNode::Text("A".into())]),
        ],
    );
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::conditional sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("conditional"), vec![vec![title]])]),
        ),
        2
    );
}

#[test]
fn dynamic_sexpr_titles_keep_distinct_source_keys() {
    let f = RdNode::tagged(
        rd_ast::RdTag::Sexpr,
        None,
        vec![RdNode::group(vec![RdNode::RCode("f()".into())])],
    );
    let g = RdNode::tagged(
        rd_ast::RdTag::Sexpr,
        None,
        vec![RdNode::group(vec![RdNode::RCode("g()".into())])],
    );
    assert_eq!(
        external_section_count(
            r#"#' @inherit pkg::f sections
#' @inherit pkg::g sections
target <- function() {}
"#,
            BTreeMap::from([
                (String::from("f"), vec![vec![f]]),
                (String::from("g"), vec![vec![g]]),
            ]),
        ),
        2
    );
}

#[test]
fn comments_do_not_change_a_section_key() {
    let title = vec![
        RdNode::Text("A".into()),
        RdNode::Comment("% ignored".into()),
    ];
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::commented sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("commented"), vec![title])]),
        ),
        1
    );
}

#[test]
fn unknown_macros_do_not_collide_with_plain_text() {
    let title = RdNode::tagged(
        rd_ast::RdTag::Unknown(r"\foo".into()),
        None,
        vec![RdNode::group(vec![RdNode::Text("A".into())])],
    );
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::unknown sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("unknown"), vec![vec![title]])]),
        ),
        2
    );
}

#[test]
fn stable_text_symbols_still_merge_with_literal_text() {
    let r = RdNode::tagged(rd_ast::RdTag::R, None, vec![]);
    let dots = RdNode::tagged(rd_ast::RdTag::Dots, None, vec![]);
    assert_eq!(
        external_section_count(
            r#"#' @section R: local body
#' @section ...: another local body
#' @inherit pkg::symbols sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("symbols"), vec![vec![r], vec![dots]],)]),
        ),
        2
    );
}

#[test]
fn quoted_titles_do_not_collide_with_plain_text() {
    let title = RdNode::tagged(rd_ast::RdTag::SQuote, None, vec![RdNode::Text("A".into())]);
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::quoted sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("quoted"), vec![vec![title]])]),
        ),
        2
    );
}

#[test]
fn known_and_unknown_spellings_of_a_tag_remain_distinct() {
    let known = RdNode::tagged(
        rd_ast::RdTag::If,
        None,
        vec![
            RdNode::group(Vec::new()),
            RdNode::group(vec![RdNode::Text("A".into())]),
        ],
    );
    let unknown = RdNode::tagged(
        rd_ast::RdTag::Unknown(r"\if".into()),
        None,
        vec![
            RdNode::group(Vec::new()),
            RdNode::group(vec![RdNode::Text("A".into())]),
        ],
    );
    assert_eq!(
        external_section_count(
            r#"#' @inherit pkg::known sections
#' @inherit pkg::unknown sections
target <- function() {}
"#,
            BTreeMap::from([
                (String::from("known"), vec![vec![known]]),
                (String::from("unknown"), vec![vec![unknown]]),
            ]),
        ),
        2
    );
}

#[test]
fn malformed_transparent_candidates_use_structural_keys() {
    let title = RdNode::tagged(
        rd_ast::RdTag::Strong,
        Some(Vec::new()),
        vec![RdNode::Text("A".into())],
    );
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::malformed sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("malformed"), vec![vec![title]])]),
        ),
        2
    );
}

#[test]
fn i_titles_merge_with_plain_text() {
    let title = RdNode::tagged(rd_ast::RdTag::I, None, vec![RdNode::Text("A".into())]);
    assert_eq!(
        external_section_count(
            r#"#' @section A: local body
#' @inherit pkg::i sections
target <- function() {}
"#,
            BTreeMap::from([(String::from("i"), vec![vec![title]])]),
        ),
        1
    );
}

#[test]
fn identical_structural_conditionals_still_merge() {
    let conditional = || {
        RdNode::tagged(
            rd_ast::RdTag::IfElse,
            None,
            vec![
                RdNode::group(vec![RdNode::Text("html".into())]),
                RdNode::group(vec![RdNode::Text("A".into())]),
                RdNode::group(vec![RdNode::Text("B".into())]),
            ],
        )
    };
    assert_eq!(
        external_section_count(
            r#"#' @inherit pkg::first sections
#' @inherit pkg::second sections
target <- function() {}
"#,
            BTreeMap::from([
                (String::from("first"), vec![vec![conditional()]]),
                (String::from("second"), vec![vec![conditional()]]),
            ]),
        ),
        1
    );
}

#[test]
fn rd_section_key_uses_link_labels_and_structures_equations() {
    let link_and_href = vec![
        RdNode::tagged(
            rd_ast::RdTag::Link,
            Some(vec![RdNode::Text("pkg:obj".into())]),
            vec![RdNode::Text("obj".into())],
        ),
        RdNode::tagged(
            rd_ast::RdTag::Href,
            None,
            vec![
                RdNode::group(vec![RdNode::Text("url".into())]),
                RdNode::group(vec![RdNode::Text("label".into())]),
            ],
        ),
    ];
    assert_eq!(
        crate::markdown_conversion::rd_section_key(&link_and_href),
        crate::markdown_conversion::rd_section_key(&[RdNode::Text("objlabel".into())]),
    );
    assert_eq!(
        crate::markdown_conversion::rd_section_key(&[link_and_href[0].clone()]),
        crate::markdown_conversion::rd_section_key(&[RdNode::tagged(
            rd_ast::RdTag::Link,
            Some(vec![RdNode::Text("other:obj".into())]),
            vec![RdNode::Text("obj".into())],
        )]),
    );

    let eqn = RdNode::tagged(
        rd_ast::RdTag::Eqn,
        None,
        vec![
            RdNode::group(vec![RdNode::Text("x^2".into())]),
            RdNode::group(vec![RdNode::Text("x squared".into())]),
        ],
    );
    let deqn = RdNode::tagged(
        rd_ast::RdTag::Deqn,
        None,
        vec![
            RdNode::group(vec![RdNode::Text("x^2".into())]),
            RdNode::group(vec![RdNode::Text("x squared".into())]),
        ],
    );
    assert_ne!(
        crate::markdown_conversion::rd_section_key(std::slice::from_ref(&eqn)),
        crate::markdown_conversion::rd_section_key(&[RdNode::Text("x squared".into())]),
    );
    assert_ne!(
        crate::markdown_conversion::rd_section_key(&[eqn]),
        crate::markdown_conversion::rd_section_key(&[deqn]),
    );
}

#[test]
fn exploratory_section_key_conversion_does_not_duplicate_final_diagnostics() {
    let source = r#"#' @name donor
#' @title Donor
#' @section A: donor body
donor <- function() {}

#' @name target
#' @title Target
#' @section # A: local body
#' @inherit donor sections
target <- function() {}
"#;
    let mut sources = SourceMap::new();
    let blocks = source_blocks(&mut sources, "test.R", source);
    let input = build_package_model(&sources, blocks);
    let inheritance = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &EmptyProvider,
        &InheritanceOptions::default(),
    );
    let output = crate::rd::build_rd(&inheritance.package, &sources);
    assert_eq!(
        output
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnsupportedMarkdownHeading)
            .count(),
        1
    );
}

#[test]
fn artificial_rcode_section_titles_merge_with_markdown_titles() {
    struct RCodeSectionProvider;

    impl DocumentationProvider for RCodeSectionProvider {
        fn get_topic(
            &self,
            request: &TopicRequest,
        ) -> Result<Option<InheritableTopic>, DocumentationError> {
            let TopicRequest::External { .. } = request else {
                return Ok(None);
            };
            let mut topic = external_title("rcode");
            topic.sections[0].title.value = InheritableContent::RCode(crate::tags::RCodeText::new(
                crate::markdown_conversion::test_support::value("A")
                    .sourced()
                    .clone(),
            ));
            Ok(Some(topic))
        }
    }

    let input = model(
        r#"#' @name target
#' @section A: local body
#' @inherit pkg::rcode sections
target <- function() {}
"#,
    );
    let options = InheritanceOptions {
        external: ExternalInheritancePolicy::BestEffort,
        external_source: ExternalPolicySource::Explicit,
    };
    let output = resolve_inheritance(
        &input.package,
        None,
        &NO_LINKS,
        &RCodeSectionProvider,
        &options,
    );
    assert_eq!(
        output.package.topics[&crate::model::TopicKey("target".into())]
            .sections
            .len(),
        1
    );
}
