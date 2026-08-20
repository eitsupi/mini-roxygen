//! Applies the single-value and duplicate policies for topic fields.
//!
//! Slot filling is separate because block-local duplicate detection and
//! topic-wide first-wins handling are shared by many tag branches.

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::Spanned;
use crate::tags::{FieldTag, FieldValue, TagOrigin, TagValue};

pub(in crate::model) fn set_single<T: Clone>(
    slot: &'static str,
    destination: &mut Option<TagValue<T>>,
    value: TagValue<T>,
    diagnostics: &mut Diagnostics,
) {
    if destination.is_none() {
        *destination = Some(value);
    } else {
        emit_duplicate(
            diagnostics,
            slot,
            value.origin.clone(),
            destination.as_ref().map(|old| old.origin.clone()),
            DuplicateSlotKind::Topic,
        );
    }
}

pub(in crate::model) fn set_field<T: Clone>(
    slot: &'static str,
    destination: &mut Option<TagValue<T>>,
    value: FieldTag<T>,
    block_slots: &mut BTreeSet<&'static str>,
    block_slot_origins: &mut BTreeMap<&'static str, TagOrigin>,
    diagnostics: &mut Diagnostics,
) {
    if !reserve_block_slot(
        slot,
        value.origin.clone(),
        block_slots,
        block_slot_origins,
        diagnostics,
    ) {
        return;
    }
    let origin = value.origin.clone();
    let FieldValue::Emit(value) = value.value else {
        return;
    };
    set_single(slot, destination, TagValue { value, origin }, diagnostics);
}

pub(in crate::model) fn set_tag<T: Clone>(
    slot: &'static str,
    destination: &mut Option<TagValue<T>>,
    value: TagValue<T>,
    block_slots: &mut BTreeSet<&'static str>,
    block_slot_origins: &mut BTreeMap<&'static str, TagOrigin>,
    diagnostics: &mut Diagnostics,
) {
    if !reserve_block_slot(
        slot,
        value.origin.clone(),
        block_slots,
        block_slot_origins,
        diagnostics,
    ) {
        return;
    }
    set_single(slot, destination, value, diagnostics);
}

fn reserve_block_slot(
    slot: &'static str,
    origin: TagOrigin,
    block_slots: &mut BTreeSet<&'static str>,
    block_slot_origins: &mut BTreeMap<&'static str, TagOrigin>,
    diagnostics: &mut Diagnostics,
) -> bool {
    if !block_slots.insert(slot) {
        emit_duplicate(
            diagnostics,
            slot,
            origin,
            block_slot_origins.get(slot).cloned(),
            DuplicateSlotKind::BlockLocal,
        );
        return false;
    }
    block_slot_origins.insert(slot, origin);
    true
}

#[derive(Debug, Clone, Copy)]
pub(in crate::model) enum DuplicateSlotKind {
    BlockLocal,
    Topic,
}

impl DuplicateSlotKind {
    pub(in crate::model) fn description(self) -> &'static str {
        match self {
            Self::BlockLocal => "block-local",
            Self::Topic => "single topic",
        }
    }
}

pub(in crate::model) fn emit_duplicate(
    diagnostics: &mut Diagnostics,
    slot: &str,
    current: TagOrigin,
    previous: Option<TagOrigin>,
    kind: DuplicateSlotKind,
) {
    let mut diagnostic = Diagnostic::new(
        Severity::Error,
        DiagnosticCode::DuplicateTag,
        format!("@{slot} fills a {} slot more than once", kind.description()),
        Label::new(
            super::origin_span(&current),
            format!("second @{slot} contribution"),
        ),
    );
    if let Some(previous) = previous {
        diagnostic = diagnostic.with_secondary(Label::new(
            super::origin_span(&previous),
            format!("first @{slot} contribution"),
        ));
    }
    diagnostics.push(diagnostic);
}

pub(in crate::model) fn emit_duplicate_method(
    diagnostics: &mut Diagnostics,
    generic: &Spanned<String>,
    class: &Spanned<String>,
    current: TagOrigin,
    previous: TagOrigin,
) {
    diagnostics.push(
        Diagnostic::new(
            Severity::Error,
            DiagnosticCode::DuplicateMethod,
            format!(
                "@method `{} {}` is declared by more than one block",
                generic.value, class.value
            ),
            Label::new(super::origin_span(&current), "second @method declaration"),
        )
        .with_secondary(Label::new(
            super::origin_span(&previous),
            "first @method declaration",
        )),
    );
}

#[cfg(test)]
mod tests {
    use crate::model::TopicKey;
    use crate::model::test_support::model;

    #[test]
    fn field_sentinels_and_structured_nulls_keep_their_distinct_meanings() {
        let suppressed = model(
            r#"#' @title Suppressed keywords
#' @keywords NULL
f <- function() f
"#,
        );
        let suppressed_topic = suppressed.package.topics[&TopicKey("f".into())].clone();
        assert!(suppressed_topic.keywords.is_empty());
        assert!(suppressed.diagnostics.is_empty());

        let ordinary = model(
            r#"#' @title Ordinary keywords
#' @keywords NULL other
f <- function() f
"#,
        );
        assert_eq!(
            ordinary.package.topics[&TopicKey("f".into())]
                .keywords
                .iter()
                .map(|keyword| keyword.0.as_str())
                .collect::<Vec<_>>(),
            ["NULL", "other"]
        );
        assert!(ordinary.diagnostics.is_empty());

        let structured = model(
            r#"#' @title Structured fields
#' @examples NULL
#' @param x NULL
#' @section NULL: body
#' @format `NULL`
f <- function(x) f
"#,
        );
        let topic = &structured.package.topics[&TopicKey("f".into())];
        let crate::tags::ExamplesContent::Ordinary(examples) =
            &topic.examples.as_ref().expect("examples").value
        else {
            panic!("expected ordinary examples");
        };
        assert_eq!(examples.as_str(), "NULL");
        assert_eq!(topic.params[0].description.as_str(), "NULL");
        assert_eq!(topic.sections[0].title.as_str(), "NULL");
        assert_eq!(
            topic.format.as_ref().expect("format").value.as_str(),
            "`NULL`"
        );
        assert!(structured.diagnostics.is_empty());
    }

    #[test]
    fn repeated_suppressed_field_is_still_a_block_local_error() {
        let output = model(
            r#"#' @title Repeated format
#' @format NULL
#' @format NULL
f <- function() f
"#,
        );
        assert!(
            output.package.topics[&TopicKey("f".into())]
                .format
                .is_none()
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(
                    |diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag
                )
                .count(),
            1
        );
    }

    #[test]
    fn parameter_repeats_are_errors_while_sections_wait_for_semantic_keys() {
        let output = model(
            r#"#' @param x first
#' @param x second
#' @section One: first
#' @section One: second
#' @aliases a a b
#' @keywords k k l
f <- function(x) x
"#,
        );
        let topic = output.package.topics.get(&TopicKey("f".into())).unwrap();
        assert_eq!(
            topic
                .aliases
                .iter()
                .map(|x| x.name.0.as_str())
                .collect::<Vec<_>>(),
            ["f", "a", "b"]
        );
        assert_eq!(
            topic
                .keywords
                .iter()
                .map(|x| x.0.as_str())
                .collect::<Vec<_>>(),
            ["k", "l"]
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code
                        == crate::diagnostic::DiagnosticCode::ConflictingParamDescription
                })
                .count(),
            1
        );
        assert_eq!(topic.sections.len(), 2);
    }

    #[test]
    fn singleton_slots_reject_a_repeat_in_one_block() {
        let output = model(
            r#"#' @name one
#' @name two
#' @rdname one
#' @rdname two
#' @title one
#' @title two
#' @description one
#' @description two
#' @details one
#' @details two
#' @return one
#' @return two
#' @usage f()
#' @usage f(x)
#' @examples one
#' @examples two
#' @order 1
#' @order 2
#' @method print one
#' @method format two
f <- function() f
"#,
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(
                    |diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag
                )
                .count(),
            10
        );
    }

    #[test]
    fn topic_slots_reject_repeats_across_merged_blocks_but_block_slots_do_not() {
        let output = model(
            r#"#' @name f
#' @rdname shared
#' @method print foo
#' @title first
#' @usage f()
f <- function() f
#' @name g
#' @rdname shared
#' @method format bar
#' @title second
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
        assert_eq!(
            output
                .diagnostics
                .iter()
                .filter(
                    |diagnostic| diagnostic.code == crate::diagnostic::DiagnosticCode::DuplicateTag
                )
                .count(),
            1
        );
    }
}
