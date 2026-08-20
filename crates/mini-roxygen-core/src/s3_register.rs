//! Static extraction of configurable S3 registrar calls.

use std::collections::BTreeSet;

use crate::arity_adapter::{CallArgumentValue, CallCallee, CallFact};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::{Span, Spanned};

/// A role occupied by one registrar argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum S3RegistrarRole {
    Generic,
    Class,
    Method,
}

/// One exact registrar call signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3RegistrarSignature {
    callee: String,
    arguments: Vec<S3RegistrarRole>,
}

impl S3RegistrarSignature {
    /// Validates a configured registrar signature.
    pub fn new(callee: impl Into<String>, arguments: Vec<S3RegistrarRole>) -> Result<Self, String> {
        let callee = callee.into();
        if callee.is_empty() {
            return Err("registrar function must not be empty".to_owned());
        }
        let mut seen = BTreeSet::new();
        for role in &arguments {
            if !seen.insert(*role as u8) {
                return Err(format!(
                    "registrar {callee:?} assigns an argument role twice"
                ));
            }
        }
        if !seen.contains(&(S3RegistrarRole::Generic as u8))
            || !seen.contains(&(S3RegistrarRole::Class as u8))
        {
            return Err(format!(
                "registrar {callee:?} must contain generic and class arguments"
            ));
        }
        Ok(Self { callee, arguments })
    }

    /// Returns the exact callee spelling matched by this signature.
    #[must_use]
    pub fn callee(&self) -> &str {
        &self.callee
    }

    /// Returns the validated argument-role order.
    #[must_use]
    pub fn arguments(&self) -> &[S3RegistrarRole] {
        &self.arguments
    }
}

/// The effective registrar signatures used by one documentation run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3RegistrarSet {
    signatures: Vec<S3RegistrarSignature>,
}

impl Default for S3RegistrarSet {
    fn default() -> Self {
        Self {
            signatures: vec![S3RegistrarSignature {
                callee: "s3_register".to_owned(),
                arguments: vec![
                    S3RegistrarRole::Generic,
                    S3RegistrarRole::Class,
                    S3RegistrarRole::Method,
                ],
            }],
        }
    }
}

impl S3RegistrarSet {
    /// Returns the built-in rlang-compatible signature plus configured additions.
    pub fn with_additions(
        additions: impl IntoIterator<Item = S3RegistrarSignature>,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        for addition in additions {
            if result
                .signatures
                .iter()
                .any(|existing| existing.callee() == addition.callee())
            {
                return Err(format!(
                    "duplicate registrar signature for {:?}",
                    addition.callee()
                ));
            }
            result.signatures.push(addition);
        }
        Ok(result)
    }

    pub fn signatures(&self) -> &[S3RegistrarSignature] {
        &self.signatures
    }
}

/// How a registration resolves to a method binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3RegistrationTarget {
    Implicit { name: String, span: Span },
    Explicit { name: String, span: Span },
    FunctionLiteral { span: Span },
    Unresolved { span: Span },
    Invalid { span: Span },
}

impl S3RegistrationTarget {
    pub(crate) fn target_span(&self) -> Span {
        match self {
            Self::Implicit { span, .. }
            | Self::Explicit { span, .. }
            | Self::FunctionLiteral { span }
            | Self::Unresolved { span }
            | Self::Invalid { span } => *span,
        }
    }
}

/// One statically proven registration pair and its target provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3RegistrationFact {
    pub generic: String,
    pub class: String,
    pub target: S3RegistrationTarget,
    pub span: Span,
}

/// Extracts all configured registrar calls from parsed syntax facts.
pub(crate) fn extract(
    calls: &[CallFact],
    registrars: &S3RegistrarSet,
) -> (Vec<S3RegistrationFact>, Diagnostics) {
    let mut facts = Vec::new();
    let mut diagnostics = Diagnostics::new();
    for call in calls {
        for signature in registrars.signatures() {
            if !matches_signature(call, signature) {
                continue;
            }
            if let Some(fact) = extract_call(call, signature, &mut diagnostics) {
                facts.push(fact);
            }
        }
    }
    (facts, diagnostics)
}

fn matches_signature(call: &CallFact, signature: &S3RegistrarSignature) -> bool {
    let Some(Spanned {
        value: Ok(callee), ..
    }) = &call.callee
    else {
        return false;
    };
    match callee {
        CallCallee::Simple(name) => name.as_str() == signature.callee(),
        CallCallee::Namespace {
            package,
            name,
            internal,
        } if !internal => signature.callee() == format!("{}::{}", package.as_str(), name.as_str()),
        CallCallee::Namespace { .. } => false,
    }
}

fn extract_call(
    call: &CallFact,
    signature: &S3RegistrarSignature,
    diagnostics: &mut Diagnostics,
) -> Option<S3RegistrationFact> {
    let mut assigned = vec![None; signature.arguments().len()];
    let mut next_positional = 0;
    for argument in &call.arguments {
        let index = if let Some(name) = &argument.name {
            let Ok(name) = &name.value else {
                registration_error(
                    diagnostics,
                    argument.value_span.unwrap_or(call.span),
                    "registrar argument name is undecodable",
                );
                return None;
            };
            let Some(index) = signature
                .arguments()
                .iter()
                .position(|role| role_name(*role) == name.as_str())
            else {
                registration_error(
                    diagnostics,
                    argument.value_span.unwrap_or(call.span),
                    "registrar uses an unknown or partial argument name",
                );
                return None;
            };
            index
        } else {
            while next_positional < assigned.len() && assigned[next_positional].is_some() {
                next_positional += 1;
            }
            if next_positional >= assigned.len() {
                registration_error(
                    diagnostics,
                    argument.value_span.unwrap_or(call.span),
                    "registrar has too many positional arguments",
                );
                return None;
            }
            let index = next_positional;
            next_positional += 1;
            index
        };
        if assigned[index].is_some() {
            registration_error(
                diagnostics,
                argument.value_span.unwrap_or(call.span),
                "registrar assigns an argument role twice",
            );
            return None;
        }
        assigned[index] = Some(argument);
    }
    let generic_argument = role_argument(
        S3RegistrarRole::Generic,
        &assigned,
        signature,
        call,
        diagnostics,
    )?;
    let class_argument = role_argument(
        S3RegistrarRole::Class,
        &assigned,
        signature,
        call,
        diagnostics,
    )?;
    // A registrar whose generic or class is computed is valid runtime code,
    // but its registration cannot be proven statically. Leave it out without
    // manufacturing either a fact or an invalid-call diagnostic.
    let has_dynamic_value =
        is_dynamic_value(&generic_argument.value) || is_dynamic_value(&class_argument.value);
    if has_dynamic_value {
        let has_static_invalid_value =
            static_role_is_invalid(S3RegistrarRole::Generic, generic_argument)
                || static_role_is_invalid(S3RegistrarRole::Class, class_argument);
        if has_static_invalid_value {
            emit_static_role_error(generic_argument, class_argument, call, diagnostics);
            return None;
        }
        dynamic_registration_info(diagnostics, call.span);
        return None;
    }
    let generic = role_value(generic_argument, call, diagnostics)?;
    let class = role_value(class_argument, call, diagnostics)?;
    let (package, generic) = generic.split_once("::").unwrap_or(("", ""));
    if package.is_empty() || generic.is_empty() || package.contains(':') || generic.contains(':') {
        registration_error(
            diagnostics,
            call.span,
            "registrar generic must be a package::generic string",
        );
        return None;
    }
    if class.is_empty() {
        registration_error(
            diagnostics,
            call.span,
            "registrar class must be a non-empty string",
        );
        return None;
    }
    let target = match signature
        .arguments()
        .iter()
        .position(|role| *role == S3RegistrarRole::Method)
        .and_then(|index| assigned[index])
        .map(|argument| (&argument.value, argument.value_span.unwrap_or(call.span)))
    {
        None | Some((CallArgumentValue::Null, _)) => S3RegistrationTarget::Implicit {
            name: format!("{generic}.{class}"),
            span: call.span,
        },
        Some((
            CallArgumentValue::Name(Spanned {
                value: Ok(name),
                span,
            }),
            _,
        )) => S3RegistrationTarget::Explicit {
            name: name.as_str().to_owned(),
            span: *span,
        },
        Some((CallArgumentValue::Function(_), span)) => {
            S3RegistrationTarget::FunctionLiteral { span }
        }
        Some((CallArgumentValue::String(_), span)) => {
            registration_error(
                diagnostics,
                span,
                "registrar method target must not be a string",
            );
            S3RegistrationTarget::Invalid { span }
        }
        Some((_, span)) => S3RegistrationTarget::Unresolved { span },
    };
    Some(S3RegistrationFact {
        generic: generic.to_owned(),
        class: class.to_owned(),
        target,
        span: call.span,
    })
}

fn role_value(
    argument: &crate::arity_adapter::CallArgument,
    call: &CallFact,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    let CallArgumentValue::String(Spanned {
        value: Ok(value), ..
    }) = &argument.value
    else {
        registration_error(
            diagnostics,
            argument.value_span.unwrap_or(call.span),
            "registrar generic and class must be string literals",
        );
        return None;
    };
    Some(value.clone())
}

fn is_dynamic_value(value: &CallArgumentValue) -> bool {
    match value {
        CallArgumentValue::Name(name) => name.value.is_ok(),
        CallArgumentValue::Other => true,
        CallArgumentValue::Function(_)
        | CallArgumentValue::String(_)
        | CallArgumentValue::Null
        | CallArgumentValue::Literal => false,
    }
}

fn static_role_is_invalid(
    role: S3RegistrarRole,
    argument: &crate::arity_adapter::CallArgument,
) -> bool {
    match &argument.value {
        CallArgumentValue::String(Spanned {
            value: Ok(value), ..
        }) => match role {
            S3RegistrarRole::Generic => !valid_generic_string(value),
            S3RegistrarRole::Class => value.is_empty(),
            S3RegistrarRole::Method => false,
        },
        _ if is_dynamic_value(&argument.value) => false,
        _ => true,
    }
}

fn emit_static_role_error(
    generic_argument: &crate::arity_adapter::CallArgument,
    class_argument: &crate::arity_adapter::CallArgument,
    call: &CallFact,
    diagnostics: &mut Diagnostics,
) {
    if static_role_is_invalid(S3RegistrarRole::Generic, generic_argument) {
        emit_role_error(
            S3RegistrarRole::Generic,
            generic_argument,
            call,
            diagnostics,
        );
    } else if static_role_is_invalid(S3RegistrarRole::Class, class_argument) {
        emit_role_error(S3RegistrarRole::Class, class_argument, call, diagnostics);
    }
}

fn emit_role_error(
    role: S3RegistrarRole,
    argument: &crate::arity_adapter::CallArgument,
    call: &CallFact,
    diagnostics: &mut Diagnostics,
) {
    match (&argument.value, role) {
        (
            CallArgumentValue::String(Spanned {
                value: Ok(value), ..
            }),
            S3RegistrarRole::Generic,
        ) if !valid_generic_string(value) => registration_error(
            diagnostics,
            call.span,
            "registrar generic must be a package::generic string",
        ),
        (
            CallArgumentValue::String(Spanned {
                value: Ok(value), ..
            }),
            S3RegistrarRole::Class,
        ) if value.is_empty() => registration_error(
            diagnostics,
            call.span,
            "registrar class must be a non-empty string",
        ),
        _ => registration_error(
            diagnostics,
            argument.value_span.unwrap_or(call.span),
            "registrar generic and class must be string literals",
        ),
    }
}

fn valid_generic_string(value: &str) -> bool {
    let (package, generic) = value.split_once("::").unwrap_or(("", ""));
    !package.is_empty() && !generic.is_empty() && !package.contains(':') && !generic.contains(':')
}

fn role_argument<'a>(
    role: S3RegistrarRole,
    assigned: &'a [Option<&'a crate::arity_adapter::CallArgument>],
    signature: &S3RegistrarSignature,
    call: &CallFact,
    diagnostics: &mut Diagnostics,
) -> Option<&'a crate::arity_adapter::CallArgument> {
    let index = signature
        .arguments()
        .iter()
        .position(|candidate| *candidate == role)?;
    let Some(argument) = assigned[index] else {
        registration_error(
            diagnostics,
            call.span,
            "registrar is missing a generic or class argument",
        );
        return None;
    };
    Some(argument)
}

fn role_name(role: S3RegistrarRole) -> &'static str {
    match role {
        S3RegistrarRole::Generic => "generic",
        S3RegistrarRole::Class => "class",
        S3RegistrarRole::Method => "method",
    }
}

fn registration_error(diagnostics: &mut Diagnostics, span: Span, message: &str) {
    diagnostics.push(Diagnostic::new(
        Severity::Error,
        DiagnosticCode::InvalidS3Registration,
        message,
        Label::new(span, "invalid S3 registrar call"),
    ));
}

fn dynamic_registration_info(diagnostics: &mut Diagnostics, span: Span) {
    diagnostics.push(
        Diagnostic::new(
            Severity::Info,
            DiagnosticCode::DynamicS3Registration,
            "dynamic S3 registrar call is delegated to runtime; no static registration fact, Rd method metadata, or NAMESPACE directive is generated",
            Label::new(span, "dynamic S3 registrar call"),
        )
        .with_help(
            "Use literal generic and class strings for static metadata, or add an explicit @method declaration.",
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        S3RegistrarRole, S3RegistrarSet, S3RegistrarSignature, S3RegistrationTarget, extract,
    };
    use crate::arity_adapter::{self, CallArgumentValue};
    use crate::diagnostic::{DiagnosticCode, Severity};
    use crate::source::{FileId, SourceFile};

    fn registrations(source: &str) -> (Vec<super::S3RegistrationFact>, usize) {
        let file = FileId::new(0);
        let parsed = arity_adapter::parse(&SourceFile::new("R/test.R".into(), source.into()), file);
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());
        (facts, diagnostics.iter().count())
    }

    #[test]
    fn finds_nested_calls_but_not_comments_or_strings() {
        let (facts, diagnostics) = registrations(
            r#"helper <- function() {
  # s3_register("bad::generic", "bad")
  "s3_register(\"bad::generic\", \"bad\")"
  s3_register("pkg::generic.with.dots", "class.with.dots")
}
"#,
        );
        assert_eq!(diagnostics, 0);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].generic, "generic.with.dots");
        assert_eq!(facts[0].class, "class.with.dots");
        assert!(
            matches!(facts[0].target, S3RegistrationTarget::Implicit { ref name, .. } if name == "generic.with.dots.class.with.dots")
        );
    }

    #[test]
    fn supports_named_reordered_arguments_and_target_shapes() {
        let signature = S3RegistrarSignature::new(
            "register_s3_method",
            vec![
                S3RegistrarRole::Class,
                S3RegistrarRole::Generic,
                S3RegistrarRole::Method,
            ],
        )
        .expect("valid signature");
        let set = S3RegistrarSet::with_additions([signature]).expect("valid set");
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"register_s3_method(class = "foo", generic = "pkg::print", method = method.foo)
register_s3_method("bar", "pkg::plot", function(x) x)
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        let (facts, diagnostics) = extract(&parsed.calls, &set);
        assert_eq!(diagnostics.iter().count(), 0);
        assert!(
            matches!(facts[0].target, S3RegistrationTarget::Explicit { ref name, .. } if name == "method.foo")
        );
        assert!(matches!(
            facts[1].target,
            S3RegistrationTarget::FunctionLiteral { .. }
        ));
    }

    #[test]
    fn malformed_generic_class_and_method_are_diagnosed_without_guessing() {
        let (facts, diagnostics) = registrations(
            r#"s3_register(pkg::generic, "class")
s3_register("pkg::generic", "")
s3_register("pkg::generic", "class", "method")
"#,
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(diagnostics, 3);
        assert!(matches!(
            facts[0].target,
            S3RegistrationTarget::Invalid { .. }
        ));
    }

    #[test]
    fn dynamic_generic_or_class_reports_info_without_fact() {
        let (facts, diagnostics) = registrations(
            r#".onLoad <- function(libname, pkgname) {
  for (class in c("class_one", "class_two")) {
    s3_register("dependency::generic", class)
  }
  s3_register(paste0("dependency::", pkgname), "class")
  s3_register("dependency::literal", "class")
}
"#,
        );
        assert_eq!(diagnostics, 2);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].generic, "literal");
        assert_eq!(facts[0].class, "class");
    }

    #[test]
    fn dynamic_values_do_not_hide_invalid_argument_structure() {
        let (facts, diagnostics) = registrations(
            r#"s3_register(generic, "class", method, extra)
s3_register(generic, generic = "pkg::generic", class = "class")
s3_register(generic)
s3_register(generic, clas = "class")
"#,
        );
        assert!(facts.is_empty());
        assert_eq!(diagnostics, 4);
    }

    #[test]
    fn dynamic_registration_reports_one_informational_diagnostic_per_call() {
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"s3_register(paste0("dependency::", generic), class)
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());

        assert!(facts.is_empty());
        let diagnostic = diagnostics.iter().next().expect("dynamic diagnostic");
        assert_eq!(diagnostics.iter().count(), 1);
        assert_eq!(diagnostic.code, DiagnosticCode::DynamicS3Registration);
        assert_eq!(diagnostic.severity, Severity::Info);
        assert_eq!(diagnostic.primary.span, parsed.calls[0].span);
        assert!(
            diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("explicit @method"))
        );
    }

    #[test]
    fn namespaced_dynamic_generic_is_other_and_reports_info() {
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"s3_register(pkg::generic, "class")
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        assert!(matches!(
            &parsed.calls[0].arguments[0].value,
            CallArgumentValue::Other
        ));
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());

        assert!(facts.is_empty());
        let diagnostic = diagnostics.iter().next().expect("dynamic diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::DynamicS3Registration);
        assert_eq!(diagnostic.severity, Severity::Info);
    }

    #[test]
    fn undecodable_name_argument_is_an_invalid_registration() {
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"s3_register(`a\b`, "class")
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        assert!(matches!(
            &parsed.calls[0].arguments[0].value,
            CallArgumentValue::Name(value) if value.value.is_err()
        ));
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());

        assert!(facts.is_empty());
        let diagnostic = diagnostics.iter().next().expect("invalid diagnostic");
        assert_eq!(diagnostics.iter().count(), 1);
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidS3Registration);
        assert_eq!(diagnostic.severity, Severity::Error);
    }

    #[test]
    fn static_non_string_variants_remain_errors_and_dynamic_variants_are_info() {
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"s3_register(1, "class")
s3_register("pkg::generic", 1)
s3_register(NULL, "class")
s3_register("pkg::generic", NULL)
s3_register(function() NULL, "class")
s3_register("pkg::generic", function() NULL)
s3_register("pkg\n::generic", "class")
s3_register("pkg::generic", "class\n")
s3_register(generic, "class")
s3_register("pkg::generic", class)
s3_register(paste0("pkg::", generic), "class")
s3_register("pkg::generic", paste0("class_", suffix))
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());

        assert!(facts.is_empty());
        let errors = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .collect::<Vec<_>>();
        let infos = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Info)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 8);
        assert_eq!(infos.len(), 4);
        assert!(
            errors
                .iter()
                .all(|diagnostic| diagnostic.code == DiagnosticCode::InvalidS3Registration)
        );
        assert!(
            infos
                .iter()
                .all(|diagnostic| diagnostic.code == DiagnosticCode::DynamicS3Registration)
        );
    }

    #[test]
    fn static_invalid_values_outrank_dynamic_values_in_either_argument_order() {
        let file = FileId::new(0);
        let source = SourceFile::new(
            "R/test.R".into(),
            r#"s3_register(generic, 1)
s3_register(1, class)
s3_register(generic, NULL)
s3_register(NULL, class)
s3_register(paste0("pkg::", generic), function() NULL)
s3_register(function() NULL, paste0("class_", suffix))
s3_register(generic, "class\n")
s3_register("pkg\n::generic", class)
s3_register(generic, `a\b`)
s3_register(`a\b`, class)
s3_register(generic, "")
s3_register("pkg:::generic", class)
"#
            .into(),
        );
        let parsed = arity_adapter::parse(&source, file);
        let (facts, diagnostics) = extract(&parsed.calls, &S3RegistrarSet::default());

        assert!(facts.is_empty());
        assert_eq!(diagnostics.iter().count(), 12);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == DiagnosticCode::InvalidS3Registration)
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == Severity::Error)
        );
    }

    #[test]
    fn generic_requires_one_exact_separator() {
        let (facts, diagnostics) = registrations(
            r#"s3_register("pkg:::generic", "class")
s3_register("pkg::::generic", "class")
s3_register("pkg::", "class")
s3_register("::generic", "class")
s3_register("pkg::generic:extra", "class")
s3_register("pkg::+", "class")
s3_register("pkg::%||%", "class")
"#,
        );
        assert_eq!(facts.len(), 2);
        assert_eq!(diagnostics, 5);
        assert_eq!(facts[0].generic, "+");
        assert_eq!(facts[1].generic, "%||%");
    }

    #[test]
    fn registrar_signatures_expose_only_validated_views() {
        assert!(S3RegistrarSignature::new("", vec![S3RegistrarRole::Generic]).is_err());
        assert!(
            S3RegistrarSignature::new(
                "custom",
                vec![S3RegistrarRole::Generic, S3RegistrarRole::Generic]
            )
            .is_err()
        );
        let set = S3RegistrarSet::default();
        let signature = &set.signatures()[0];
        assert_eq!(signature.callee(), "s3_register");
        assert_eq!(
            signature.arguments(),
            &[
                S3RegistrarRole::Generic,
                S3RegistrarRole::Class,
                S3RegistrarRole::Method,
            ]
        );
    }
}
