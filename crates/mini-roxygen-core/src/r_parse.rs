//! Builds a file-scoped object index from adapter IR.
//!
//! This layer only associates documented blocks with the syntax facts of their
//! paired top-level expressions. It does not interpret tags or evaluate R, so
//! later layers can decide how a documented syntax shape becomes a topic.

use crate::arity_adapter::{
    AssignmentFact, AssignmentOperator, AssignmentTarget, AssignmentValue, BindingName, BlockId,
    CallFact, Formal, FormalError, ParsedFile, RName, RNameDecodeError, S7ClassAnalysis,
    S7ClassFact, S7ClassRefusal, TopLevelFact, TopLevelShape,
};
use crate::source::{FileId, Span, Spanned};

/// The syntax-only objects documented in one source file, in source order.
///
/// The index is deliberately scoped to one file because binding identities and
/// documentation associations are established from source order before any
/// package-wide name resolution takes place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileObjectIndex {
    /// The file whose top-level expressions produced this index.
    pub file: FileId,
    /// Documented objects in the order in which their expressions occur.
    pub documented: Vec<DocumentedObject>,
    /// Every top-level simple binding assignment, including undocumented ones.
    pub bindings: Vec<BindingFact>,
}

/// One package-local top-level binding assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingFact {
    /// The decoded binding name and its source spelling.
    pub name: BindingName,
    /// The assignment operator.
    pub operator: AssignmentOperator,
    /// The complete assignment span.
    pub assignment_span: Span,
    /// The function's formals when this binding directly receives a function.
    pub function_formals: Option<Result<Vec<Formal>, FormalError>>,
    /// The statically established category of the assigned value.
    pub value: BindingValue,
}

/// The conservative categories used by the package-local binding resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingValue {
    /// A direct function expression and its static generic property.
    Function {
        /// Whether the function body calls unqualified `UseMethod`.
        calls_use_method: bool,
    },
    /// A decoded simple-name alias.
    Alias(RName),
    /// A statically extracted S7 class constructor.
    S7Class(S7ClassFact),
    /// A recognized S7 class whose metadata is statically unsupported.
    S7Refused(S7ClassRefusal),
    /// A value statically known not to be a function.
    NonFunction,
    /// A value whose function-ness cannot be established without evaluation.
    Unknown,
}

/// One documented top-level expression and the syntax object it can provide.
///
/// The block identity is retained instead of using the vector position so
/// later topic construction can merge results without confusing source order
/// with the file-local identity assigned by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentedObject {
    /// The file-local identity of the documentation block.
    pub block: BlockId,
    /// The source span of the documentation block.
    pub block_span: Span,
    /// The syntax-only target associated with the block.
    pub target: BlockTarget,
}

/// The supported syntax shapes of a documented top-level expression.
///
/// This enum keeps function assignments separate from other assignments so a
/// `ValueObject` cannot accidentally carry function-only metadata. Opaque
/// calls and typed refusals remain available to a later semantic layer without
/// making this layer guess what they mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockTarget {
    /// A simple binding assigned a function expression.
    FunctionAssignment(FunctionObject),
    /// A simple binding assigned a non-function expression.
    ValueAssignment(ValueObject),
    /// The bare `NULL` expression.
    Null {
        /// The source span of the `NULL` expression.
        span: Span,
    },
    /// A top-level string literal naming a data object.
    DataObject(DataObject),
    /// The package-level documentation sentinel.
    PackageDocumentation(PackageSentinel),
    /// A top-level call retained without interpreting its callee or arguments.
    Call(CallFact),
    /// A syntax shape that association can retain but cannot classify further.
    Refused(AssociationRefusal),
}

/// Metadata for a function assigned to a simple binding.
///
/// The adapter has already handled parenthesized function values, so this
/// object can expose formals and body provenance without evaluating the right
/// hand side or re-reading the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionObject {
    /// The binding name and its original spelling span.
    pub name: BindingName,
    /// The operator used by the assignment.
    pub operator: AssignmentOperator,
    /// The span of the complete assignment expression.
    pub assignment_span: Span,
    /// The span of the function expression on the right-hand side.
    pub value_span: Span,
    /// The function's formal parameters, or a structural extraction failure.
    pub formals: Result<Vec<Formal>, FormalError>,
    /// The direct function body span, when the adapter could identify it.
    pub body_span: Option<Span>,
    /// Whether the function body calls unqualified `UseMethod`.
    pub calls_use_method: bool,
}

/// Metadata for a non-function value assigned to a simple binding.
///
/// Keeping this as a separate type makes it impossible for consumers to treat
/// a function as an ordinary value merely because both came from an
/// assignment expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueObject {
    /// The binding name and its original spelling span.
    pub name: BindingName,
    /// The operator used by the assignment.
    pub operator: AssignmentOperator,
    /// The span of the complete assignment expression.
    pub assignment_span: Span,
    /// The span of the right-hand-side expression.
    pub value_span: Span,
    /// The syntax-only category of the non-function value.
    pub value: NonFunctionValue,
}

/// A data object named by a top-level string literal.
///
/// Data names are kept separate from [`RName`] because string literals may
/// name datasets with values that are not valid bare R binding names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataObject {
    /// The decoded data name and its source span.
    pub name: Spanned<DataName>,
}

/// The decoded name of a dataset documented by a string literal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DataName(String);

impl DataName {
    /// Returns the data name without imposing R's bare-name grammar.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The source span of a recognised package-documentation sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageSentinel {
    /// The source span of the `_PACKAGE` string literal.
    pub span: Span,
}

/// The non-function assignment values needed by later static processing.
///
/// This is intentionally not an alias for the adapter's `AssignmentValue`:
/// that enum also contains `Function`, while this layer's value object must
/// make that contradictory state unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonFunctionValue {
    /// A statically decoded name used as the right-hand side.
    Name(Spanned<Result<RName, RNameDecodeError>>),
    /// A statically extracted S7 class constructor.
    S7Class(S7ClassFact),
    /// A recognized S7 class whose metadata is statically unsupported.
    S7Refused(S7ClassRefusal),
    /// An opaque call used as the right-hand side.
    Call(CallFact),
    /// A literal value, including R's reserved constants.
    Literal,
    /// Any other non-function expression.
    Other,
}

/// A syntactic association that needs a later layer to decide whether manual
/// documentation can recover a topic from it.
///
/// These are records, rather than diagnostics, because a block can provide
/// explicit metadata such as `@name` and `@usage` that makes a refused syntax
/// a legitimate manual topic. This parser layer must not make that semantic
/// decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssociationRefusal {
    /// An assignment through a replacement function or other compound target.
    CompoundAssignment {
        /// The source span of the compound target.
        target_span: Span,
    },
    /// An assignment target that R does not accept as a binding.
    InvalidAssignmentTarget {
        /// The source span of the invalid target.
        target_span: Span,
    },
    /// A binding spelling whose R escape grammar the adapter could not decode.
    UndecodableBinding {
        /// The source span of the undecodable target.
        target_span: Span,
        /// The reason decoding was refused.
        reason: RNameDecodeError,
    },
    /// A data-object string literal could not be decoded without R's escape
    /// grammar.
    UndecodableDataName {
        /// The source span of the string literal.
        span: Span,
        /// The reason decoding was refused.
        reason: RNameDecodeError,
    },
    /// A data-object string literal decoded to an empty name.
    EmptyDataName {
        /// The source span of the string literal.
        span: Span,
    },
    /// An expression shape for which this layer has no object model.
    UnsupportedExpression {
        /// The source span of the complete unsupported expression.
        span: Span,
    },
}

/// Builds the object index for one file from the adapter's parse result.
///
/// Documentation entries are retained separately from all simple bindings.
/// Adapter diagnostics are deliberately not converted or re-emitted here: this layer
/// creates no diagnostics and only projects syntax facts into object IR. Tag
/// contents are likewise left untouched because `@name`, `@rdname`, and other
/// tags affect later topic construction, not syntactic association.
#[must_use]
pub fn build_object_index(parsed: ParsedFile, file: FileId) -> FileObjectIndex {
    let mut documented = Vec::new();
    let mut bindings = Vec::new();
    for entry in parsed.top_level {
        if let TopLevelShape::Assignment(assignment) = &entry.fact.shape
            && let AssignmentTarget::Binding(_) = &assignment.target
            && matches!(
                assignment.operator,
                AssignmentOperator::Left | AssignmentOperator::Equals | AssignmentOperator::Right
            )
        {
            bindings.push(binding_fact(entry.fact.span, assignment));
        }
        let Some(block) = entry.documentation else {
            continue;
        };
        let target = block_target(entry.fact);
        documented.push(DocumentedObject {
            block: block.id,
            block_span: block.span,
            target,
        });
    }

    FileObjectIndex {
        file,
        documented,
        bindings,
    }
}

fn binding_fact(assignment_span: Span, assignment: &AssignmentFact) -> BindingFact {
    let function_formals = match &assignment.value {
        AssignmentValue::Function(function) => Some(function.formals.clone()),
        _ => None,
    };
    let value = match &assignment.value {
        AssignmentValue::Function(function) => BindingValue::Function {
            calls_use_method: function.calls_use_method,
        },
        AssignmentValue::Name(name) => match &name.value {
            Ok(name) => BindingValue::Alias(name.clone()),
            Err(_) => BindingValue::Unknown,
        },
        AssignmentValue::Literal => BindingValue::NonFunction,
        AssignmentValue::Call(call) => match &call.s7_class {
            S7ClassAnalysis::Supported(class) => BindingValue::S7Class(class.clone()),
            S7ClassAnalysis::Refused(refusal) => BindingValue::S7Refused(refusal.clone()),
            S7ClassAnalysis::NotApplicable => BindingValue::Unknown,
        },
        AssignmentValue::Other => BindingValue::Unknown,
    };
    let AssignmentTarget::Binding(name) = &assignment.target else {
        unreachable!("binding facts are created only for simple binding targets");
    };
    BindingFact {
        name: name.clone(),
        operator: assignment.operator,
        assignment_span,
        function_formals,
        value,
    }
}

fn block_target(fact: TopLevelFact) -> BlockTarget {
    let TopLevelFact { span, shape } = fact;
    match shape {
        TopLevelShape::Assignment(assignment) => assignment_target(assignment, span),
        TopLevelShape::Null => BlockTarget::Null { span },
        TopLevelShape::StringLiteral(value) => {
            let Spanned { value, span } = value;
            match value {
                Ok(name) if name == "_PACKAGE" => {
                    BlockTarget::PackageDocumentation(PackageSentinel { span })
                }
                Ok(name) if name.is_empty() => {
                    BlockTarget::Refused(AssociationRefusal::EmptyDataName { span })
                }
                Ok(name) => BlockTarget::DataObject(DataObject {
                    name: Spanned::new(DataName(name), span),
                }),
                Err(reason) => {
                    BlockTarget::Refused(AssociationRefusal::UndecodableDataName { span, reason })
                }
            }
        }
        TopLevelShape::Call(call) => BlockTarget::Call(call),
        TopLevelShape::Other => {
            BlockTarget::Refused(AssociationRefusal::UnsupportedExpression { span })
        }
    }
}

fn assignment_target(assignment: AssignmentFact, assignment_span: Span) -> BlockTarget {
    let AssignmentFact {
        operator,
        target,
        target_span,
        value,
        value_span,
    } = assignment;

    match target {
        // Each non-function value is named here rather than caught by a
        // wildcard, so adding a variant to the adapter's value enum is a
        // compile error to be decided rather than a silent `Other`.
        AssignmentTarget::Binding(name) => {
            let value = match value {
                AssignmentValue::Function(function) => {
                    return BlockTarget::FunctionAssignment(FunctionObject {
                        name,
                        operator,
                        assignment_span,
                        value_span,
                        formals: function.formals,
                        body_span: function.body_span,
                        calls_use_method: function.calls_use_method,
                    });
                }
                AssignmentValue::Name(name) => NonFunctionValue::Name(name),
                AssignmentValue::Call(call) => match call.s7_class.clone() {
                    S7ClassAnalysis::Supported(class) => NonFunctionValue::S7Class(class),
                    S7ClassAnalysis::Refused(refusal) => NonFunctionValue::S7Refused(refusal),
                    S7ClassAnalysis::NotApplicable => NonFunctionValue::Call(call),
                },
                AssignmentValue::Literal => NonFunctionValue::Literal,
                AssignmentValue::Other => NonFunctionValue::Other,
            };
            BlockTarget::ValueAssignment(ValueObject {
                name,
                operator,
                assignment_span,
                value_span,
                value,
            })
        }
        AssignmentTarget::Compound { .. } => {
            BlockTarget::Refused(AssociationRefusal::CompoundAssignment { target_span })
        }
        AssignmentTarget::Invalid { .. } => {
            BlockTarget::Refused(AssociationRefusal::InvalidAssignmentTarget { target_span })
        }
        AssignmentTarget::Undecodable { reason, .. } => {
            BlockTarget::Refused(AssociationRefusal::UndecodableBinding {
                target_span,
                reason,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        AssociationRefusal, BlockTarget, FileObjectIndex, NonFunctionValue, build_object_index,
    };
    use crate::arity_adapter::parse;
    use crate::source::{FileId, SourceFile};

    const FILE: FileId = FileId::new(11);

    fn index(source_text: &str) -> FileObjectIndex {
        let source = SourceFile::new(PathBuf::from("test.R"), source_text.to_owned());
        build_object_index(parse(&source, FILE), FILE)
    }

    #[test]
    fn indexes_documented_function_and_reaches_formals() {
        let index = index(
            r#"#' documented
f <- (function(x, y = 1) x)
"#,
        );
        assert_eq!(index.documented.len(), 1);
        let BlockTarget::FunctionAssignment(function) = &index.documented[0].target else {
            panic!("expected a function assignment");
        };
        assert_eq!(function.name.canonical.as_str(), "f");
        let formals = function.formals.as_ref().expect("valid formals");
        assert_eq!(formals.len(), 2);
        assert_eq!(formals[0].name.value.as_ref().unwrap().as_str(), "x");
        assert_eq!(formals[1].name.value.as_ref().unwrap().as_str(), "y");
        assert!(function.body_span.is_some());
    }

    #[test]
    fn indexes_documented_non_function_without_function_state() {
        let index = index(
            r#"#' documented
value <- other
"#,
        );
        let BlockTarget::ValueAssignment(value) = &index.documented[0].target else {
            panic!("expected a value assignment");
        };
        assert_eq!(value.name.canonical.as_str(), "value");
        let NonFunctionValue::Name(name) = &value.value else {
            panic!("expected a name value");
        };
        assert_eq!(name.value.as_ref().unwrap().as_str(), "other");
    }

    #[test]
    fn documented_assignment_targets_retain_every_operator() {
        for (source_text, expected) in [
            (
                "#' documented\nleft <- function() NULL\n",
                crate::arity_adapter::AssignmentOperator::Left,
            ),
            (
                "#' documented\nright = function() NULL\n",
                crate::arity_adapter::AssignmentOperator::Equals,
            ),
            (
                "#' documented\n(function() NULL) -> right\n",
                crate::arity_adapter::AssignmentOperator::Right,
            ),
            (
                "#' documented\nleft <<- function() NULL\n",
                crate::arity_adapter::AssignmentOperator::SuperLeft,
            ),
            (
                "#' documented\n(function() NULL) ->> right\n",
                crate::arity_adapter::AssignmentOperator::SuperRight,
            ),
            (
                "#' documented\nleft := function() NULL\n",
                crate::arity_adapter::AssignmentOperator::Walrus,
            ),
        ] {
            let index = index(source_text);
            assert_eq!(index.documented.len(), 1, "{source_text}");
            let operator = match &index.documented[0].target {
                BlockTarget::FunctionAssignment(function) => function.operator,
                other => panic!(
                    "expected documented function assignment for {source_text:?}, got {other:?}"
                ),
            };
            assert_eq!(operator, expected, "{source_text}");
        }
    }

    #[test]
    fn indexes_documented_null_and_package_string_without_interpreting_string() {
        let null_index = index(
            r#"#' documented
NULL
"#,
        );
        assert!(matches!(
            null_index.documented[0].target,
            BlockTarget::Null { .. }
        ));

        let package_index = index(
            r#"#' documented
"_PACKAGE"
"#,
        );
        let BlockTarget::PackageDocumentation(value) = &package_index.documented[0].target else {
            panic!("expected package documentation sentinel");
        };
        assert_eq!(value.span.range.len(), 10);
    }

    #[test]
    fn indexes_documented_call_as_opaque_fact() {
        let index = index(
            r#"#' documented
make_object(value)
"#,
        );
        let BlockTarget::Call(call) = &index.documented[0].target else {
            panic!("expected a call");
        };
        assert!(call.callee.is_some());
    }

    #[test]
    fn records_each_typed_association_refusal() {
        let cases = [
            (
                r#"#' documented
x$y <- 1
"#,
                "compound",
            ),
            (
                r#"#' documented
NULL <- 1
"#,
                "invalid",
            ),
            (
                r#"#' documented
"a\x2e b" <- 1
"#,
                "undecodable",
            ),
            (
                r#"#' documented
1 + 2
"#,
                "unsupported",
            ),
        ];

        for (source_text, expected) in cases {
            let index = index(source_text);
            let BlockTarget::Refused(refusal) = &index.documented[0].target else {
                panic!("expected {expected} refusal");
            };
            match (expected, refusal) {
                ("compound", AssociationRefusal::CompoundAssignment { .. })
                | ("invalid", AssociationRefusal::InvalidAssignmentTarget { .. })
                | ("undecodable", AssociationRefusal::UndecodableBinding { .. })
                | ("unsupported", AssociationRefusal::UnsupportedExpression { .. }) => {}
                _ => panic!("wrong refusal variant for {expected}: {refusal:?}"),
            }
        }
    }

    #[test]
    fn omits_undocumented_expressions() {
        let index = index(
            r#"#' documented
first <- 1
second <- 2
"#,
        );
        assert_eq!(index.documented.len(), 1);
        let BlockTarget::ValueAssignment(value) = &index.documented[0].target else {
            panic!("expected a value assignment");
        };
        assert_eq!(value.name.canonical.as_str(), "first");
    }

    #[test]
    fn preserves_source_order_and_each_block_id() {
        let index = index(
            r#"#' first
first <- 1
#' second
second <- 2
"#,
        );
        assert_eq!(index.documented.len(), 2);
        assert_eq!(index.documented[0].block.index(), 0);
        assert_eq!(index.documented[1].block.index(), 1);
        let BlockTarget::ValueAssignment(first) = &index.documented[0].target else {
            panic!("expected first value assignment");
        };
        let BlockTarget::ValueAssignment(second) = &index.documented[1].target else {
            panic!("expected second value assignment");
        };
        assert_eq!(first.name.canonical.as_str(), "first");
        assert_eq!(second.name.canonical.as_str(), "second");
        assert_ne!(index.documented[0].block, index.documented[1].block);
    }
}
