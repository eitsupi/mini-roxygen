//! Static substitutions for inline R expressions.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use rd_ast::{RdDocument, RdNode, RdTag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::{FileId, Span, Spanned, TextRange};

/// The validated, effective substitutions used while converting Markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineRSubstitutions {
    entries: BTreeMap<String, InlineRSubstitution>,
    user_keys: BTreeSet<String>,
    origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct InlineRSubstitution {
    nodes: Vec<RdNode>,
    definition_spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InlineRMatch {
    pub(crate) nodes: Vec<RdNode>,
    pub(crate) definition_spans: Vec<Span>,
}

/// Records user substitutions matched during one documentation invocation.
#[derive(Debug)]
pub(crate) struct InlineRUsage {
    matched_user_keys: RefCell<BTreeSet<String>>,
}

/// Pairs immutable substitutions with invocation-local usage tracking.
pub(crate) struct InlineRSession<'a> {
    substitutions: &'a InlineRSubstitutions,
    usage: &'a InlineRUsage,
}

impl InlineRUsage {
    pub(crate) fn new() -> Self {
        Self {
            matched_user_keys: RefCell::new(BTreeSet::new()),
        }
    }

    fn record(&self, key: &str) {
        self.matched_user_keys.borrow_mut().insert(key.to_owned());
    }
}

impl<'a> InlineRSession<'a> {
    pub(crate) fn new(substitutions: &'a InlineRSubstitutions, usage: &'a InlineRUsage) -> Self {
        Self {
            substitutions,
            usage,
        }
    }

    pub(crate) fn lookup(&self, key: &str) -> Option<InlineRMatch> {
        let result = self.substitutions.lookup(key);
        if result.is_some() && self.substitutions.user_keys.contains(key) {
            self.usage.record(key);
        }
        result
    }
}

impl InlineRSubstitutions {
    /// Builds the built-in substitution table.
    pub fn builtins() -> Result<Self, String> {
        let mut table = Self {
            entries: BTreeMap::new(),
            user_keys: BTreeSet::new(),
            origin: None,
        };
        for stage in LIFECYCLE_BADGE_STAGES {
            let key = format!(r#"lifecycle::badge("{stage}")"#);
            let value = lifecycle_badge_fragment(stage);
            let nodes = validate_fragment(&key, &value)?;
            table.entries.insert(
                key.to_owned(),
                InlineRSubstitution {
                    nodes,
                    definition_spans: Vec::new(),
                },
            );
        }
        Ok(table)
    }

    /// Builds an effective table from user-provided Rd fragments.
    ///
    /// Every user value is validated before any replacement is committed. If
    /// validation fails, all invalid values are returned as diagnostics and no
    /// table is returned.
    pub fn from_user_entries(
        entries: BTreeMap<String, String>,
        origin: Option<String>,
    ) -> Result<Self, Diagnostics> {
        let entries = entries
            .into_iter()
            .map(|(key, value)| (key, (value, None)))
            .collect();
        Self::from_user_entries_internal(entries, origin)
    }

    /// Builds an effective table while retaining each user's definition span.
    pub fn from_user_entries_with_spans(
        entries: BTreeMap<String, Spanned<String>>,
        origin: Option<String>,
    ) -> Result<Self, Diagnostics> {
        let entries = entries
            .into_iter()
            .map(|(key, value)| (key, (value.value, Some(value.span))))
            .collect();
        Self::from_user_entries_internal(entries, origin)
    }

    fn from_user_entries_internal(
        entries: BTreeMap<String, (String, Option<Span>)>,
        origin: Option<String>,
    ) -> Result<Self, Diagnostics> {
        let mut table = match Self::builtins() {
            Ok(table) => table,
            Err(reason) => {
                let mut diagnostics = Diagnostics::new();
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagnosticCode::InvalidInlineRSubstitution,
                        format!("invalid built-in inline R substitution: {reason}"),
                        Label::new(unresolvable_span(), "invalid inline R substitution"),
                    )
                    .with_help("provide valid Rd fragments for inline R substitutions"),
                );
                return Err(diagnostics);
            }
        };
        let mut validated = BTreeMap::new();
        let mut diagnostics = Diagnostics::new();
        for (key, (value, definition_span)) in entries {
            match validate_fragment(&key, &value) {
                Ok(nodes) => {
                    validated.insert(key, (nodes, definition_span));
                }
                Err(reason) => {
                    let origin = origin
                        .as_deref()
                        .map_or_else(String::new, |origin| format!(" in {origin}"));
                    let primary = definition_span.unwrap_or_else(unresolvable_span);
                    diagnostics.push(
                        Diagnostic::new(
                            Severity::Error,
                            DiagnosticCode::InvalidInlineRSubstitution,
                            format!(
                                "invalid Rd substitution for inline R expression {key:?}{origin}: {reason}"
                            ),
                            Label::new(primary, "invalid inline R substitution"),
                        )
                        .with_help("provide a valid Rd fragment for this exact inline R expression"),
                    );
                }
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        for (key, (nodes, definition_span)) in validated {
            table.entries.insert(
                key.clone(),
                InlineRSubstitution {
                    nodes,
                    definition_spans: definition_span.into_iter().collect(),
                },
            );
            table.user_keys.insert(key);
        }
        table.origin = origin;
        Ok(table)
    }

    /// Looks up an exact expression key without recording usage.
    pub(crate) fn lookup(&self, key: &str) -> Option<InlineRMatch> {
        self.entries.get(key).map(|entry| InlineRMatch {
            nodes: entry.nodes.clone(),
            definition_spans: entry.definition_spans.clone(),
        })
    }

    /// Reports user entries that were never encountered in source Markdown.
    pub(crate) fn unused_diagnostics(&self, usage: &InlineRUsage) -> Diagnostics {
        let matched = usage.matched_user_keys.borrow();
        let mut diagnostics = Diagnostics::new();
        for key in self.user_keys.difference(&matched) {
            let origin = self
                .origin
                .as_deref()
                .map_or_else(String::new, |origin| format!(" in {origin}"));
            diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    DiagnosticCode::UnusedInlineRSubstitution,
                    format!("inline R substitution for {key:?}{origin} was not used"),
                    Label::new(
                        self.entries[key]
                            .definition_spans
                            .first()
                            .copied()
                            .unwrap_or_else(unresolvable_span),
                        "unused inline R substitution",
                    ),
                )
                .with_help("remove the unused inline R substitution"),
            );
        }
        diagnostics
    }
}

// The stage vocabulary and badge shape follow lifecycle's public source:
// https://github.com/r-lib/lifecycle. See THIRD_PARTY_NOTICES.md.
const LIFECYCLE_BADGE_STAGES: [&str; 9] = [
    "experimental",
    "stable",
    "superseded",
    "deprecated",
    "maturing",
    "questioning",
    "soft-deprecated",
    "defunct",
    "retired",
];

fn lifecycle_badge_fragment(stage: &str) -> String {
    let mut characters = stage.chars();
    let first = characters
        .next()
        .expect("lifecycle badge stages must not be empty");
    let label = first.to_uppercase().chain(characters).collect::<String>();
    let url = format!("https://lifecycle.r-lib.org/articles/stages.html#{stage}");
    let image = format!("lifecycle-{stage}.svg");
    let html = [
        r"\href{",
        url.as_str(),
        r"}{\figure{",
        image.as_str(),
        r"}{options: alt='[",
        label.as_str(),
        r"]'}}",
    ]
    .concat();
    let text = [r"\strong{[", label.as_str(), r"]}"].concat();
    [r"\ifelse{html}{", html.as_str(), "}{", text.as_str(), "}"].concat()
}

fn unresolvable_span() -> Span {
    // FileId(0) resolves to the first R source file. Use an out-of-range file
    // identity for diagnostics that intentionally have no source location.
    Span::new(FileId::new(u32::MAX), TextRange::new(0, 0))
}

fn rd_diagnostic_message(diagnostic: &rd_source::Diagnostic, wrapper_len: usize) -> String {
    let start = diagnostic.span().bytes().start;
    match start.checked_sub(wrapper_len) {
        Some(offset) => format!(
            "Rd parser diagnostic: {} at byte {offset}",
            diagnostic.message()
        ),
        None => format!("Rd parser diagnostic: {}", diagnostic.message()),
    }
}

fn validate_fragment(key: &str, value: &str) -> Result<Vec<RdNode>, String> {
    let wrapped = format!(r"\description{{{value}}}");
    let parsed = rd_source::parse(wrapped.as_bytes())
        .map_err(|error| format!("Rd parser error: {error}"))?;
    if let Some(diagnostic) = parsed.diagnostics().first() {
        return Err(rd_diagnostic_message(diagnostic, r"\description{".len()));
    }
    let Some(node) = parsed.document().nodes().first() else {
        return Err("parser produced no wrapper tag".to_owned());
    };
    if parsed.document().nodes().len() != 1 {
        return Err("parser produced nodes outside the wrapper tag".to_owned());
    }
    let Some(tagged) = node.as_tagged() else {
        return Err("parser did not produce the description wrapper tag".to_owned());
    };
    if tagged.tag() != &RdTag::Description {
        return Err(format!(
            "parser produced unexpected wrapper tag {}",
            tagged.tag().as_rd_tag()
        ));
    }
    let children = tagged.children().to_vec();
    let fragment = RdDocument::from(children.clone());
    if let Err(error) = rd_writer::write_document(&fragment) {
        let location = error
            .ast_path()
            .and_then(|path| fragment_writer_extent(path, parsed.source_map()))
            .and_then(|extent| extent.start.checked_sub(r"\description{".len()))
            .map_or_else(String::new, |offset| format!(" at byte {offset}"));
        return Err(format!("Rd writer error{location}: {error}"));
    }
    let _ = key;
    Ok(children)
}

fn fragment_writer_extent(
    path: &rd_ast::RdAstPath,
    source_map: &rd_source::RdSourceMap,
) -> Option<std::ops::Range<usize>> {
    use rd_ast::RdAstPathSegment;

    let [RdAstPathSegment::TopLevel(index), rest @ ..] = path.segments() else {
        return None;
    };
    let mut segments = vec![
        RdAstPathSegment::TopLevel(0),
        RdAstPathSegment::Child(*index),
    ];
    for segment in rest {
        match segment {
            RdAstPathSegment::Child(_) | RdAstPathSegment::Option => {
                segments.push(segment.clone());
            }
            RdAstPathSegment::TopLevel(_) => return None,
            _ => return None,
        }
    }
    let mapped = rd_ast::RdAstPath::new(segments);
    source_map.span(&mapped).map(|span| span.bytes())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rd_ast::{RdAstPath, RdAstPathSegment, RdDocument};
    use rd_writer::Writer;

    use super::{InlineRSession, InlineRSubstitutions, InlineRUsage, fragment_writer_extent};
    use crate::diagnostic::DiagnosticCode;
    use crate::source::{Spanned, TextRange};

    #[test]
    fn fragment_writer_paths_project_through_the_wrapper_source_map() {
        let input = r"\description{\link[alt]{\strong{nested}}\link[]{second}}";
        let parsed = rd_source::parse(input.as_bytes()).expect("fixture should parse");
        let first = RdAstPath::new(vec![RdAstPathSegment::TopLevel(0)]);
        let second = RdAstPath::new(vec![RdAstPathSegment::TopLevel(1)]);

        let assert_text = |path: &RdAstPath, expected: &str| {
            let extent = fragment_writer_extent(path, parsed.source_map()).expect("mapped span");
            assert_eq!(&input.as_bytes()[extent], expected.as_bytes());
        };
        assert_text(&first, r"\link[alt]{\strong{nested}}");
        assert_text(&second, r"\link[]{second}");
        assert_text(&first.with_option(), "[alt]");
        assert_text(&second.with_option(), "[]");
        assert_text(&first.with_child(0), r"\strong{nested}");
        assert_text(&first.with_child(0).with_child(0), "nested");

        assert!(fragment_writer_extent(&RdAstPath::new(Vec::new()), parsed.source_map()).is_none());
        assert!(
            fragment_writer_extent(
                &RdAstPath::new(vec![RdAstPathSegment::Child(0)]),
                parsed.source_map()
            )
            .is_none()
        );
        assert!(
            fragment_writer_extent(
                &RdAstPath::new(vec![
                    RdAstPathSegment::TopLevel(0),
                    RdAstPathSegment::TopLevel(0),
                ]),
                parsed.source_map()
            )
            .is_none()
        );
        assert!(
            fragment_writer_extent(
                &RdAstPath::new(vec![RdAstPathSegment::TopLevel(99)]),
                parsed.source_map()
            )
            .is_none()
        );
        assert!(fragment_writer_extent(&first.with_child(99), parsed.source_map()).is_none());
        assert!(
            fragment_writer_extent(&second.with_option().with_option(), parsed.source_map())
                .is_none()
        );
    }

    #[test]
    fn lifecycle_badges_match_the_rendered_fragments_for_all_stages() {
        let table = InlineRSubstitutions::builtins().expect("built-ins should validate");
        let expected = [
            (
                "experimental",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#experimental}{\figure{lifecycle-experimental.svg}{options: alt='[Experimental]'}}}{\strong{[Experimental]}}"#,
            ),
            (
                "stable",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#stable}{\figure{lifecycle-stable.svg}{options: alt='[Stable]'}}}{\strong{[Stable]}}"#,
            ),
            (
                "superseded",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#superseded}{\figure{lifecycle-superseded.svg}{options: alt='[Superseded]'}}}{\strong{[Superseded]}}"#,
            ),
            (
                "deprecated",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#deprecated}{\figure{lifecycle-deprecated.svg}{options: alt='[Deprecated]'}}}{\strong{[Deprecated]}}"#,
            ),
            (
                "maturing",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#maturing}{\figure{lifecycle-maturing.svg}{options: alt='[Maturing]'}}}{\strong{[Maturing]}}"#,
            ),
            (
                "questioning",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#questioning}{\figure{lifecycle-questioning.svg}{options: alt='[Questioning]'}}}{\strong{[Questioning]}}"#,
            ),
            (
                "soft-deprecated",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#soft-deprecated}{\figure{lifecycle-soft-deprecated.svg}{options: alt='[Soft-deprecated]'}}}{\strong{[Soft-deprecated]}}"#,
            ),
            (
                "defunct",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#defunct}{\figure{lifecycle-defunct.svg}{options: alt='[Defunct]'}}}{\strong{[Defunct]}}"#,
            ),
            (
                "retired",
                r#"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#retired}{\figure{lifecycle-retired.svg}{options: alt='[Retired]'}}}{\strong{[Retired]}}"#,
            ),
        ];

        for (stage, expected) in expected {
            let key = format!(r#"lifecycle::badge("{stage}")"#);
            let output = Writer::new(rd_writer::WriterOptions::default())
                .write_document(&RdDocument::from(
                    table.lookup(&key).expect("lifecycle badge stage").nodes,
                ))
                .expect("badge fragment should serialize");
            assert_eq!(output, expected, "rendered fragment for {stage}");
        }
    }

    #[test]
    fn built_in_badge_is_a_writer_round_trippable_rd_fragment() {
        let table = InlineRSubstitutions::builtins().expect("built-ins should validate");
        let nodes = table
            .lookup(r#"lifecycle::badge("experimental")"#)
            .expect("experimental badge")
            .nodes;
        let output = Writer::new(rd_writer::WriterOptions::default())
            .write_document(&RdDocument::from(nodes))
            .expect("badge fragment should serialize");
        assert!(output.contains(
            r"\ifelse{html}{\href{https://lifecycle.r-lib.org/articles/stages.html#experimental}"
        ));
    }

    #[test]
    fn user_entries_override_built_ins_and_empty_values_are_mappings() {
        let table = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([
                (
                    r#"lifecycle::badge("experimental")"#.to_owned(),
                    r#"\strong{custom}"#.to_owned(),
                ),
                ("empty()".to_owned(), String::new()),
            ]),
            Some("configuration label".to_owned()),
        )
        .expect("configuration should validate");
        let overridden = table
            .lookup(r#"lifecycle::badge("experimental")"#)
            .expect("overridden badge");
        assert_eq!(
            Writer::new(rd_writer::WriterOptions::default())
                .write_document(&RdDocument::from(overridden.nodes))
                .expect("override should serialize"),
            r"\strong{custom}"
        );
        assert!(
            table
                .lookup("empty()")
                .expect("empty mapping")
                .nodes
                .is_empty()
        );
    }

    #[test]
    fn malformed_rd_is_reported_as_a_core_diagnostic() {
        let diagnostics = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("broken()".to_owned(), r#"\strong{"#.to_owned())]),
            Some("configuration label".to_owned()),
        )
        .expect_err("invalid Rd should be a diagnostic");
        let diagnostic = diagnostics.iter().next().expect("invalid diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidInlineRSubstitution);
        assert!(diagnostic.message.contains("broken()"));
        assert!(diagnostic.message.contains("Rd parser"));
        assert!(diagnostic.message.contains("unclosed group"));
        assert!(!diagnostic.message.contains(".R"));
    }

    #[test]
    fn invalid_user_definition_uses_its_configuration_span() {
        let span = crate::source::Span::new(crate::source::FileId::new(7), TextRange::new(11, 19));
        let diagnostics = InlineRSubstitutions::from_user_entries_with_spans(
            BTreeMap::from([(
                "broken()".to_owned(),
                Spanned::new(r#"\strong{"#.to_owned(), span),
            )]),
            Some("mini-roxygen.toml".to_owned()),
        )
        .expect_err("invalid Rd should be a diagnostic");
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").primary.span,
            span
        );
    }

    #[test]
    fn invalid_rd_offset_is_relative_to_the_configured_value() {
        let diagnostics = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("broken()".to_owned(), "prefix }".to_owned())]),
            None,
        )
        .expect_err("invalid Rd should be a diagnostic");
        let message = &diagnostics
            .iter()
            .next()
            .expect("invalid diagnostic")
            .message;
        assert!(message.contains("unexpected closing delimiter"));
        assert!(message.contains("at byte 8"));
    }

    #[test]
    fn multiline_literal_values_keep_both_lines() {
        let table = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([(
                "lines()".to_owned(),
                "\n\\code{first}\n\\code{second}\n".to_owned(),
            )]),
            None,
        )
        .expect("multiline value should validate");
        let output = Writer::new(rd_writer::WriterOptions::default())
            .write_document(&RdDocument::from(
                table.lookup("lines()").expect("lines mapping").nodes,
            ))
            .expect("multiline value should serialize");
        assert!(output.contains(r"\code{first}"));
        assert!(output.contains(r"\code{second}"));
    }

    #[test]
    fn basic_string_escapes_become_the_intended_rd_fragment() {
        let table = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("escaped()".to_owned(), r#"\code{1}"#.to_owned())]),
            None,
        )
        .expect("basic string should validate");
        let output = Writer::new(rd_writer::WriterOptions::default())
            .write_document(&RdDocument::from(
                table.lookup("escaped()").expect("escaped mapping").nodes,
            ))
            .expect("escaped value should serialize");
        assert_eq!(output, r"\code{1}");
    }

    #[test]
    fn unused_user_entries_warn_but_built_ins_do_not() {
        let table = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("never()".to_owned(), r#"\code{never}"#.to_owned())]),
            Some("user configuration".to_owned()),
        )
        .expect("configuration should validate");
        let usage = InlineRUsage::new();
        let diagnostics = table.unused_diagnostics(&usage);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("unused diagnostic").code,
            DiagnosticCode::UnusedInlineRSubstitution
        );
        assert!(
            diagnostics
                .iter()
                .next()
                .expect("unused diagnostic")
                .message
                .contains("user configuration")
        );
        assert!(
            !diagnostics
                .iter()
                .next()
                .expect("unused diagnostic")
                .message
                .contains(".R")
        );

        let builtins = InlineRSubstitutions::builtins().expect("built-ins should validate");
        assert!(builtins.unused_diagnostics(&InlineRUsage::new()).is_empty());
    }

    #[test]
    fn unused_user_definition_uses_its_configuration_span() {
        let span = crate::source::Span::new(crate::source::FileId::new(3), TextRange::new(4, 12));
        let table = InlineRSubstitutions::from_user_entries_with_spans(
            BTreeMap::from([(
                "never()".to_owned(),
                Spanned::new(r#"\code{never}"#.to_owned(), span),
            )]),
            None,
        )
        .expect("configuration should validate");
        let diagnostics = table.unused_diagnostics(&InlineRUsage::new());
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").primary.span,
            span
        );
    }

    #[test]
    fn usage_records_are_independent_between_invocations() {
        let table = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([("known()".to_owned(), r#"\strong{known}"#.to_owned())]),
            None,
        )
        .expect("configuration should validate");
        let matched_usage = InlineRUsage::new();
        let unmatched_usage = InlineRUsage::new();
        let session = InlineRSession::new(&table, &matched_usage);

        assert!(session.lookup("known()").is_some());
        assert_eq!(table.unused_diagnostics(&unmatched_usage).len(), 1);
        assert!(table.unused_diagnostics(&matched_usage).is_empty());
    }

    #[test]
    fn several_invalid_entries_are_all_reported_without_a_partial_table() {
        let diagnostics = InlineRSubstitutions::from_user_entries(
            BTreeMap::from([
                ("first()".to_owned(), r#"\strong{"#.to_owned()),
                ("second()".to_owned(), "prefix }".to_owned()),
            ]),
            Some("config".to_owned()),
        )
        .expect_err("all invalid values should be rejected");
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == DiagnosticCode::InvalidInlineRSubstitution)
        );
    }
}
