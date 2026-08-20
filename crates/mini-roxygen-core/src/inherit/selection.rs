//! Evaluates the supported argument-selection language used by inheritance tags.

use std::collections::BTreeMap;

use crate::tags::{ArgSelection, ArgSelector, ParamName};

/// A structured failure while evaluating one selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionError {
    /// The semantic kind of failure.
    pub kind: SelectionErrorKind,
    /// The source span of the selector that failed.
    pub span: crate::source::Span,
}

/// The semantic failures recognized by the pure selector evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionErrorKind {
    /// A selector named a parameter absent from the supplied domain.
    UnknownName(ParamName),
}

/// Evaluates a name-only argument selection over an ordered parameter domain.
///
/// The returned names always use domain order. The evaluator does not remove
/// `...`; excluding that name is the caller's responsibility because this
/// function only interprets the domain it receives. Duplicate domain names
/// are treated by their first occurrence.
pub fn evaluate_selection(
    domain: &[ParamName],
    selection: &ArgSelection,
) -> Result<Vec<ParamName>, SelectionError> {
    let mut positions = BTreeMap::new();
    let mut unique_domain = Vec::new();
    for name in domain {
        if !positions.contains_key(&name.0) {
            let index = unique_domain.len();
            positions.insert(name.0.clone(), index);
            unique_domain.push(name.clone());
        }
    }

    if selection.selectors.is_empty() {
        return Ok(unique_domain);
    }

    let first_is_exclusion = matches!(selection.selectors[0], ArgSelector::Exclude(_));
    let mut selected = vec![first_is_exclusion; unique_domain.len()];
    for selector in &selection.selectors {
        let (name, include) = match selector {
            ArgSelector::Name(name) => (&name.value, true),
            ArgSelector::Exclude(name) => (&name.value, false),
        };
        let Some(&index) = positions.get(&name.0) else {
            return Err(SelectionError {
                kind: SelectionErrorKind::UnknownName(name.clone()),
                span: match selector {
                    ArgSelector::Name(name) | ArgSelector::Exclude(name) => name.span,
                },
            });
        };
        selected[index] = include;
    }

    Ok(unique_domain
        .into_iter()
        .zip(selected)
        .filter_map(|(name, selected)| selected.then_some(name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{SelectionErrorKind, evaluate_selection};
    use crate::source::{FileId, Span, Spanned, TextRange};
    use crate::tags::{ArgSelection, ArgSelector, ParamName};

    fn span(start: u32) -> Span {
        Span::new(FileId::new(0), TextRange::new(start, start + 1))
    }

    fn name(value: &str, start: u32) -> ArgSelector {
        ArgSelector::Name(Spanned::new(ParamName(value.to_owned()), span(start)))
    }

    fn exclude(value: &str, start: u32) -> ArgSelector {
        ArgSelector::Exclude(Spanned::new(ParamName(value.to_owned()), span(start)))
    }

    fn selection(selectors: Vec<ArgSelector>) -> ArgSelection {
        ArgSelection { selectors }
    }

    fn domain(names: &[&str]) -> Vec<ParamName> {
        names.iter().map(|name| ParamName((*name).into())).collect()
    }

    #[test]
    fn empty_selection_returns_the_domain() {
        assert_eq!(
            evaluate_selection(&domain(&["x", "y"]), &selection(vec![])).unwrap(),
            domain(&["x", "y"])
        );
    }

    #[test]
    fn positive_first_selector_starts_empty() {
        assert_eq!(
            evaluate_selection(&domain(&["x", "y", "z"]), &selection(vec![name("y", 1)])).unwrap(),
            domain(&["y"])
        );
    }

    #[test]
    fn negative_first_selector_starts_with_the_full_domain() {
        assert_eq!(
            evaluate_selection(&domain(&["x", "y", "z"]), &selection(vec![exclude("y", 1)]),)
                .unwrap(),
            domain(&["x", "z"])
        );
    }

    #[test]
    fn later_selectors_reverse_add_and_remove_in_both_orders() {
        assert_eq!(
            evaluate_selection(
                &domain(&["x", "y", "z"]),
                &selection(vec![exclude("y", 1), name("y", 2)]),
            )
            .unwrap(),
            domain(&["x", "y", "z"])
        );
        assert_eq!(
            evaluate_selection(
                &domain(&["x", "y", "z"]),
                &selection(vec![name("y", 1), exclude("y", 2)]),
            )
            .unwrap(),
            domain(&[])
        );
    }

    #[test]
    fn selection_reports_unknown_names() {
        let error =
            evaluate_selection(&domain(&["x"]), &selection(vec![name("missing", 7)])).unwrap_err();
        assert!(matches!(error.kind, SelectionErrorKind::UnknownName(_)));
        assert_eq!(error.span, span(7));
    }

    #[test]
    fn duplicate_domain_names_are_selected_once_in_domain_order() {
        let result = evaluate_selection(
            &domain(&["b", "a", "b", "c"]),
            &selection(vec![name("c", 1), name("a", 2)]),
        )
        .unwrap();
        assert_eq!(result, domain(&["a", "c"]));
    }

    #[test]
    fn ellipsis_is_selected_when_the_caller_keeps_it_in_the_domain() {
        let result = evaluate_selection(
            &domain(&["x", "...", "y"]),
            &selection(vec![name("...", 1)]),
        )
        .unwrap();
        assert_eq!(result, domain(&["..."]));
    }
}
