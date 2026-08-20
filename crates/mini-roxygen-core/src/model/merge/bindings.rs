use std::collections::{BTreeMap, BTreeSet};

use crate::arity_adapter::{S7ClassFact, S7ClassRefusal};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::r_parse::{BindingFact, BindingValue};
use crate::s3_register::S3RegistrationFact;
use crate::source::{SourceMap, Span, Spanned};
use crate::tags::TagOrigin;

use super::super::{FormalNames, MethodDeclaration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum S7BindingResolution {
    Supported(S7ClassFact),
    Refused(S7ClassRefusal),
}

pub(super) fn s7_binding_results(
    bindings: &[BindingFact],
) -> BTreeMap<Span, Option<S7BindingResolution>> {
    let mut ordered = bindings.to_vec();
    ordered.sort_by_key(|binding| binding.assignment_span);
    let mut known = BTreeMap::<String, S7BindingResolution>::new();
    let mut results = BTreeMap::new();
    let mut current_file = None;
    for binding in ordered {
        if current_file != Some(binding.assignment_span.file) {
            known.clear();
            current_file = Some(binding.assignment_span.file);
        }
        let value = match &binding.value {
            BindingValue::S7Class(class) => Some(S7BindingResolution::Supported(class.clone())),
            BindingValue::S7Refused(refusal) => Some(S7BindingResolution::Refused(refusal.clone())),
            BindingValue::Alias(alias) => known.get(alias.as_str()).cloned(),
            BindingValue::Function { .. } | BindingValue::NonFunction | BindingValue::Unknown => {
                None
            }
        };
        if let Some(resolution) = &value {
            known.insert(
                binding.name.canonical.as_str().to_owned(),
                resolution.clone(),
            );
        } else {
            known.remove(binding.name.canonical.as_str());
        }
        results.insert(binding.assignment_span, value);
    }
    results
}

pub(super) fn alias_formal_results(
    bindings: &[BindingFact],
    sources: &SourceMap,
    collate: bool,
) -> BTreeMap<Span, FormalNames> {
    bindings
        .iter()
        .map(|binding| {
            let mut visiting = BTreeSet::new();
            let names = binding_formal_names(binding, bindings, sources, collate, &mut visiting);
            (binding.assignment_span, names)
        })
        .collect()
}

pub(super) fn alias_function_formal_results(
    bindings: &[BindingFact],
    sources: &SourceMap,
    collate: bool,
) -> BTreeMap<
    Span,
    Option<Result<Vec<crate::arity_adapter::Formal>, crate::arity_adapter::FormalError>>,
> {
    bindings
        .iter()
        .map(|binding| {
            let mut visiting = BTreeSet::new();
            let formals =
                binding_function_formals(binding, bindings, sources, collate, &mut visiting);
            (binding.assignment_span, formals)
        })
        .collect()
}

fn binding_function_formals(
    binding: &BindingFact,
    bindings: &[BindingFact],
    sources: &SourceMap,
    collate: bool,
    visiting: &mut BTreeSet<String>,
) -> Option<Result<Vec<crate::arity_adapter::Formal>, crate::arity_adapter::FormalError>> {
    match &binding.value {
        BindingValue::Function { .. } => binding.function_formals.clone(),
        BindingValue::Alias(target) => {
            if !visiting.insert(binding.name.canonical.as_str().to_owned()) {
                return None;
            }
            let candidates = bindings
                .iter()
                .filter(|candidate| candidate.name.canonical == *target)
                .collect::<Vec<_>>();
            let result = if candidates.len() == 1
                && binding_precedes_alias(candidates[0], binding, sources, collate)
            {
                binding_function_formals(candidates[0], bindings, sources, collate, visiting)
            } else {
                None
            };
            visiting.remove(binding.name.canonical.as_str());
            result
        }
        BindingValue::NonFunction
        | BindingValue::S7Class(_)
        | BindingValue::S7Refused(_)
        | BindingValue::Unknown => None,
    }
}

fn binding_formal_names(
    binding: &BindingFact,
    bindings: &[BindingFact],
    sources: &SourceMap,
    collate: bool,
    visiting: &mut BTreeSet<String>,
) -> FormalNames {
    match &binding.value {
        BindingValue::Function { .. } => binding
            .function_formals
            .as_ref()
            .map(|formals| {
                super::super::usage::formal_names_from_formals(formals, binding.assignment_span)
            })
            .unwrap_or(FormalNames::Unknown {
                span: binding.assignment_span,
            }),
        BindingValue::Alias(target) => {
            if !visiting.insert(binding.name.canonical.as_str().to_owned()) {
                return FormalNames::Unknown {
                    span: binding.assignment_span,
                };
            }
            let candidates = bindings
                .iter()
                .filter(|candidate| candidate.name.canonical == *target)
                .collect::<Vec<_>>();
            let result = if candidates.len() == 1
                && binding_precedes_alias(candidates[0], binding, sources, collate)
            {
                binding_formal_names(candidates[0], bindings, sources, collate, visiting)
            } else {
                FormalNames::Unknown {
                    span: binding.assignment_span,
                }
            };
            visiting.remove(binding.name.canonical.as_str());
            result
        }
        BindingValue::NonFunction
        | BindingValue::S7Class(_)
        | BindingValue::S7Refused(_)
        | BindingValue::Unknown => FormalNames::NotFunction,
    }
}

fn binding_precedes_alias(
    target: &BindingFact,
    alias: &BindingFact,
    sources: &SourceMap,
    collate: bool,
) -> bool {
    if target.assignment_span.file == alias.assignment_span.file {
        target.assignment_span.range.start() < alias.assignment_span.range.start()
    } else {
        !collate
            && sources
                .compare_filename_order(target.assignment_span.file, alias.assignment_span.file)
                .is_some_and(|ordering| ordering.is_lt())
    }
}

pub(super) fn validate_registration_targets(
    registrations: &[S3RegistrationFact],
    bindings: &[BindingFact],
    diagnostics: &mut Diagnostics,
) {
    for registration in registrations {
        let Some(name) = registration_target_name(&registration.target) else {
            continue;
        };
        let matching = bindings
            .iter()
            .filter(|binding| binding.name.canonical.as_str() == name)
            .collect::<Vec<_>>();
        let valid =
            matching.len() == 1 && binding_is_function(matching[0], bindings, &mut BTreeSet::new());
        if !valid {
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                DiagnosticCode::InvalidS3Registration,
                format!("S3 registrar target `{name}` is missing or is not a local function"),
                Label::new(
                    registration.target.target_span(),
                    "registration target is not a function",
                ),
            ));
        }
    }
}

fn binding_is_function(
    binding: &BindingFact,
    bindings: &[BindingFact],
    visiting: &mut BTreeSet<String>,
) -> bool {
    match &binding.value {
        BindingValue::Function { .. } => true,
        BindingValue::Alias(target)
            if visiting.insert(binding.name.canonical.as_str().to_owned()) =>
        {
            let candidates = bindings
                .iter()
                .filter(|candidate| candidate.name.canonical.as_str() == target.as_str())
                .collect::<Vec<_>>();
            candidates.len() == 1 && binding_is_function(candidates[0], bindings, visiting)
        }
        BindingValue::Alias(_) | BindingValue::NonFunction | BindingValue::Unknown => false,
        BindingValue::S7Class(_) | BindingValue::S7Refused(_) => false,
    }
}

pub(super) fn registration_target_name(
    target: &crate::s3_register::S3RegistrationTarget,
) -> Option<&str> {
    match target {
        crate::s3_register::S3RegistrationTarget::Implicit { name, .. }
        | crate::s3_register::S3RegistrationTarget::Explicit { name, .. } => Some(name),
        crate::s3_register::S3RegistrationTarget::FunctionLiteral { .. }
        | crate::s3_register::S3RegistrationTarget::Unresolved { .. }
        | crate::s3_register::S3RegistrationTarget::Invalid { .. } => None,
    }
}

pub(super) fn registration_for_target<'a>(
    registrations: &'a [S3RegistrationFact],
    name: &str,
) -> Option<&'a S3RegistrationFact> {
    let matches = registration_matches(registrations, name);
    (matches.len() == 1).then(|| matches[0])
}

pub(super) fn registration_matches<'a>(
    registrations: &'a [S3RegistrationFact],
    name: &str,
) -> Vec<&'a S3RegistrationFact> {
    let mut matches = Vec::new();
    for registration in registrations
        .iter()
        .filter(|registration| registration_target_name(&registration.target) == Some(name))
    {
        if matches.iter().any(|previous: &&S3RegistrationFact| {
            previous.generic == registration.generic && previous.class == registration.class
        }) {
            continue;
        }
        matches.push(registration);
    }
    matches
}

pub(super) fn registration_method(registration: &S3RegistrationFact) -> MethodDeclaration {
    MethodDeclaration {
        generic: Spanned::new(registration.generic.clone(), registration.span),
        class: Spanned::new(registration.class.clone(), registration.span),
        origin: TagOrigin::Implicit {
            intro_span: registration.span,
        },
    }
}
