//! Resolves usage tags and statically generated function usage.
//!
//! Usage has its own provenance variants and generation errors, so keeping it
//! separate lets merge logic only decide when resolution should occur.

use crate::arity_adapter::{CallCallee, S7ClassFact};
use crate::r_parse::{BlockTarget, FunctionObject, NonFunctionValue, ValueObject};
use crate::source::SourceMap;
use crate::tags::{ParamName, TagValue, UsageDirective};
use crate::usage::{GeneratedUsage, UsageError, generate_function_usage};

use super::{FormalName, FormalNames, ResolvedUsage};

pub(in crate::model) fn resolve_formal_names(
    target: &BlockTarget,
    s7_class: Option<&S7ClassFact>,
    alias_formals: Option<&FormalNames>,
) -> FormalNames {
    let function = match target {
        BlockTarget::FunctionAssignment(function) => function,
        BlockTarget::ValueAssignment(ValueObject {
            value:
                NonFunctionValue::Name(_)
                | NonFunctionValue::S7Class(_)
                | NonFunctionValue::S7Refused(_),
            ..
        }) if let Some(class) = s7_class => {
            return formal_names_from_formals(&class.constructor.formals, class.class_name.span);
        }
        BlockTarget::ValueAssignment(ValueObject {
            value: NonFunctionValue::Name(_),
            ..
        }) => return alias_formals.cloned().unwrap_or(FormalNames::NotFunction),
        _ => return FormalNames::NotFunction,
    };

    formal_names_from_formals(&function.formals, function.value_span)
}

pub(in crate::model) fn formal_names_from_formals(
    formals: &Result<Vec<crate::arity_adapter::Formal>, crate::arity_adapter::FormalError>,
    fallback_span: crate::source::Span,
) -> FormalNames {
    let formals = match formals {
        Ok(formals) => formals,
        Err(_) => {
            return FormalNames::Unknown {
                span: fallback_span,
            };
        }
    };

    let mut names = Vec::with_capacity(formals.len());
    for formal in formals {
        let Ok(name) = &formal.name.value else {
            return FormalNames::Undecodable {
                span: formal.name.span,
            };
        };
        names.push(FormalName {
            name: ParamName(name.as_str().to_owned()),
            span: formal.name.span,
        });
    }
    FormalNames::Known(names)
}

pub(in crate::model) fn resolve_usage(
    target: &BlockTarget,
    sources: &SourceMap,
    s7_class: Option<&S7ClassFact>,
    alias_function_formals: Option<
        &Result<Vec<crate::arity_adapter::Formal>, crate::arity_adapter::FormalError>,
    >,
    lazy_data: bool,
) -> Result<Option<GeneratedUsage>, UsageError> {
    if let BlockTarget::DataObject(data) = target {
        return Ok(Some(GeneratedUsage::data(
            data.name.value.as_str(),
            lazy_data,
        )));
    }
    let function = match target {
        BlockTarget::FunctionAssignment(function) => Some(function.clone()),
        BlockTarget::ValueAssignment(value) => match &value.value {
            NonFunctionValue::Name(_) => alias_function_formals
                .map(|formals| FunctionObject {
                    name: value.name.clone(),
                    operator: value.operator,
                    assignment_span: value.assignment_span,
                    value_span: value.value_span,
                    formals: (*formals).clone(),
                    body_span: None,
                    calls_use_method: false,
                })
                .or_else(|| {
                    s7_class.map(|class| FunctionObject {
                        name: value.name.clone(),
                        operator: value.operator,
                        assignment_span: value.assignment_span,
                        value_span: value.value_span,
                        formals: class.constructor.formals.clone(),
                        body_span: None,
                        calls_use_method: false,
                    })
                }),
            NonFunctionValue::S7Class(_) | NonFunctionValue::S7Refused(_) => {
                s7_class.map(|class| FunctionObject {
                    name: value.name.clone(),
                    operator: value.operator,
                    assignment_span: value.assignment_span,
                    value_span: value.value_span,
                    formals: class.constructor.formals.clone(),
                    body_span: None,
                    calls_use_method: false,
                })
            }
            NonFunctionValue::Call(call) if is_known_non_function_constructor(call) => {
                return Ok(Some(GeneratedUsage::object(value.name.canonical.as_str())));
            }
            NonFunctionValue::Call(_) => None,
            NonFunctionValue::Literal | NonFunctionValue::Other => None,
        },
        _ => None,
    };
    if let Some(function) = function {
        match generate_function_usage(&function, sources) {
            Ok(usage) => Ok(Some(usage)),
            Err(error) => Err(error),
        }
    } else {
        Ok(None)
    }
}

fn is_known_non_function_constructor(call: &crate::arity_adapter::CallFact) -> bool {
    matches!(
        call.callee.as_ref().and_then(|callee| callee.value.as_ref().ok()),
        Some(CallCallee::Namespace {
            package,
            name,
            internal: _,
        }) if package.as_str() == "base" && name.as_str() == "new.env"
    )
}

pub(in crate::model) fn resolve_explicit_usage(value: &TagValue<UsageDirective>) -> ResolvedUsage {
    match &value.value {
        UsageDirective::SuppressGenerated => ResolvedUsage::Suppressed(value.origin.clone()),
        UsageDirective::Explicit(source) => ResolvedUsage::Explicit(TagValue {
            value: source.clone(),
            origin: value.origin.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::ResolvedUsage;
    use crate::model::TopicKey;
    use crate::model::test_support::model;

    #[test]
    fn explicit_usage_and_null_suppression_keep_provenance_variants() {
        let explicit = model(
            r#"#' @title Explicit usage
#' @usage f(custom)
f <- function(x) x
"#,
        );
        let topic = explicit.package.topics.get(&TopicKey("f".into())).unwrap();
        assert!(matches!(
            topic.usages.as_slice(),
            [usage] if matches!(usage.usage, ResolvedUsage::Explicit(_))
        ));
        assert!(explicit.diagnostics.is_empty());

        let suppressed = model(
            r#"#' @title Suppressed usage
#' @usage NULL
f <- function(x) x
"#,
        );
        let topic = suppressed
            .package
            .topics
            .get(&TopicKey("f".into()))
            .unwrap();
        assert!(matches!(
            topic.usages.as_slice(),
            [usage] if matches!(usage.usage, ResolvedUsage::Suppressed(_))
        ));
        assert!(suppressed.diagnostics.is_empty());
    }
}
