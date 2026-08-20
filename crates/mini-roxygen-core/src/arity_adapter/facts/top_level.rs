//! Classifies top-level expression shapes and their binding facts.
//!
//! This module is separate because expression classification composes decoded names and function shapes into the public syntax facts without changing either lower-level responsibility.

use arity_parser::ast::{
    Arg, AssignmentExpr, AstNode, AstToken, BinaryExpr, CallExpr, Expr, HasArgList, RConstant,
};
use arity_parser::syntax::{SyntaxElement, SyntaxKind};

use crate::source::{FileId, Span, Spanned};

use super::super::window::TopLevelExpr;
use super::function::FunctionFact;
use super::name::{RName, RNameDecodeError};
use super::{
    decode_string_literal, function_fact, name_delimiter, span_for_element, span_for_expression,
    span_for_node, span_for_token,
};
/// The syntax facts for one top-level R expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopLevelFact {
    /// The complete source span of the top-level expression.
    pub span: Span,
    /// The expression's syntax-only shape.
    pub shape: TopLevelShape,
}

/// The supported top-level expression shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopLevelShape {
    /// An assignment, including its direction and syntactic target/value
    /// shapes.
    Assignment(AssignmentFact),
    /// A function call whose callee is statically recognizable, when it is a
    /// simple name or namespace-qualified name.
    Call(CallFact),
    /// The bare `NULL` constant.
    Null,
    /// A string literal, with its decoded value (or the reason it was refused)
    /// and source span. A string value may legitimately be empty, which is
    /// why this is not an [`RName`].
    StringLiteral(Spanned<Result<String, RNameDecodeError>>),
    /// Any other top-level expression.
    Other,
}

/// Syntax facts for an assignment expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentFact {
    /// The assignment operator, independent of arity's syntax kinds.
    pub operator: AssignmentOperator,
    /// The target's decoded name, compound shape, or decoding limitation.
    pub target: AssignmentTarget,
    /// The source span of the target, including a compound target when there
    /// is no simple binding.
    pub target_span: Span,
    /// The value's syntax shape and any facts specific to that shape.
    pub value: AssignmentValue,
    /// The source span of the value expression.
    pub value_span: Span,
}

/// The syntax shape of an assignment value, with shape-specific facts carried
/// by the variants that need them. Keeping the shape and its facts together
/// prevents the contradictory states allowed by the former separate shape and
/// optional-function fields, while retaining a decoded right-hand-side name
/// makes alias assignments such as `b <- a` representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentValue {
    /// Function formals and body provenance, including a function wrapped in
    /// parentheses.
    Function(FunctionFact),
    /// The decoded right-hand-side name and its source span, or a typed refusal
    /// when the spelling needs R escape handling that this adapter does not
    /// implement yet.
    Name(Spanned<Result<RName, RNameDecodeError>>),
    /// Facts for a call expression, including its complete source span.
    Call(CallFact),
    /// A literal value, including reserved R constants.
    Literal,
    /// Any other value expression.
    Other,
}
/// The R assignment operators represented by mini-roxygen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssignmentOperator {
    Left,
    Equals,
    SuperLeft,
    Right,
    SuperRight,
    Walrus,
}
/// A canonical R binding name and the span of its original spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingName {
    /// The decoded R binding name.
    pub canonical: RName,
    /// The target span exactly as written in the source.
    pub spelling: Span,
}

/// The target of an assignment, retaining the distinction between syntax we
/// intentionally skip and a name this adapter could not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentTarget {
    /// A simple binding whose name decoded exactly.
    Binding(BindingName),
    /// A valid target that binds through a replacement function rather than a
    /// name, such as `x$y`, `x[[1]]`, or `dim(x)`.
    Compound { span: Span },
    /// A target R itself rejects, such as a bare reserved constant or a
    /// number. R reports these as an invalid left-hand side to assignment.
    Invalid { span: Span },
    /// A name this adapter cannot decode yet.
    Undecodable {
        span: Span,
        reason: RNameDecodeError,
    },
}

/// A statically recognizable call callee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallCallee {
    Simple(RName),
    Namespace {
        package: RName,
        name: RName,
        internal: bool,
    },
}

/// Syntax facts for a top-level call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFact {
    /// The complete source span of the call.
    pub span: Span,
    /// The decoded simple or namespace-qualified callee, or a typed decoding
    /// refusal. Its span covers the callee portion of the source.
    pub callee: Option<Spanned<Result<CallCallee, RNameDecodeError>>>,
    /// Direct call arguments, retained for the small set of static call
    /// shapes whose metadata is needed by later layers.
    pub arguments: Vec<CallArgument>,
    /// Typed analysis of an S7 `new_class` call, including source-aware
    /// refusals for recognized but unsupported shapes.
    pub s7_class: S7ClassAnalysis,
}

/// The result of classifying a call as an S7 constructor shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S7ClassAnalysis {
    /// The call is not one of the supported `new_class` spellings.
    NotApplicable,
    /// The call has a literal class name and direct constructor function.
    Supported(S7ClassFact),
    /// The call is `new_class` but its static metadata is unsupported.
    Refused(S7ClassRefusal),
}

/// A source-backed reason why a recognized S7 class could not be analyzed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ClassRefusal {
    /// The unsupported class-name or constructor source span.
    pub span: Span,
    /// The static shape that was refused.
    pub reason: S7ClassRefusalReason,
}

/// Static refusal reasons for S7 class metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S7ClassRefusalReason {
    /// The first argument is not a literal string class name.
    ComputedClassName,
    /// The constructor argument is absent.
    MissingConstructor,
    /// The constructor argument is not a direct function expression.
    ComputedConstructor,
}

/// One call argument with parser-independent syntax facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallArgument {
    /// The decoded argument name, when the argument is named.
    pub name: Option<Spanned<Result<RName, RNameDecodeError>>>,
    /// The argument value's supported syntax shape.
    pub value: CallArgumentValue,
    /// The value expression span.
    pub value_span: Option<crate::source::Span>,
}

/// A call argument value retained without evaluating R.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallArgumentValue {
    /// A direct function expression and its constructor formals.
    Function(FunctionFact),
    /// A decoded string literal.
    String(Spanned<Result<String, RNameDecodeError>>),
    /// A decoded simple name.
    Name(Spanned<Result<RName, RNameDecodeError>>),
    /// The syntactic `NULL` constant.
    Null,
    /// A literal that does not carry a name.
    Literal,
    /// Any unsupported or computed expression.
    Other,
}

/// Static metadata extracted from a supported S7 `new_class` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ClassFact {
    /// The literal class name passed as the first positional argument.
    pub class_name: Spanned<String>,
    /// The named constructor's formals.
    pub constructor: FunctionFact,
}
pub(in crate::arity_adapter) fn top_level_facts(
    expressions: &[TopLevelExpr],
    file_id: FileId,
) -> Vec<TopLevelFact> {
    expressions
        .iter()
        .map(|expression| fact_for_expression(expression.expression.clone(), file_id))
        .collect()
}

pub(in crate::arity_adapter) fn nested_call_facts(
    expressions: &[TopLevelExpr],
    file_id: FileId,
) -> Vec<CallFact> {
    let mut calls = expressions
        .iter()
        .flat_map(|expression| match expression.expression.syntax() {
            SyntaxElement::Node(node) => node
                .descendants()
                .filter_map(|node| {
                    CallExpr::cast(node).map(|call| {
                        let span = span_for_node(call.syntax(), file_id);
                        call_fact(call, file_id, span)
                    })
                })
                .collect::<Vec<_>>(),
            SyntaxElement::Token(_) => Vec::new(),
        })
        .collect::<Vec<_>>();
    calls.sort_by_key(|call| call.span);
    calls
}

fn fact_for_expression(expression: Expr, file_id: FileId) -> TopLevelFact {
    let span = span_for_expression(&expression, file_id);
    let shape = match expression {
        Expr::Assignment(assignment) => assignment_fact(&assignment, file_id, span)
            .map(TopLevelShape::Assignment)
            .unwrap_or(TopLevelShape::Other),
        Expr::Call(call) => TopLevelShape::Call(call_fact(call, file_id, span)),
        Expr::Name(identifier) if identifier.constant() == Some(RConstant::Null) => {
            TopLevelShape::Null
        }
        Expr::StringLiteral(literal) => {
            TopLevelShape::StringLiteral(Spanned::new(decode_string_literal(&literal), span))
        }
        _ => TopLevelShape::Other,
    };
    TopLevelFact { span, shape }
}

fn assignment_fact(
    assignment: &AssignmentExpr,
    file_id: FileId,
    assignment_span: Span,
) -> Option<AssignmentFact> {
    let operator = match assignment.op_kind()? {
        SyntaxKind::ASSIGN_LEFT => AssignmentOperator::Left,
        SyntaxKind::ASSIGN_EQ => AssignmentOperator::Equals,
        SyntaxKind::SUPER_ASSIGN => AssignmentOperator::SuperLeft,
        SyntaxKind::ASSIGN_RIGHT => AssignmentOperator::Right,
        SyntaxKind::SUPER_ASSIGN_RIGHT => AssignmentOperator::SuperRight,
        SyntaxKind::WALRUS => AssignmentOperator::Walrus,
        _ => return None,
    };
    let target_element = assignment.target_element();
    let value_element = assignment.value_element();
    let target_span = target_element
        .as_ref()
        .map(|element| span_for_element(element, file_id))
        .unwrap_or(assignment_span);
    let value_element = value_element?;
    let value_span = span_for_element(&value_element, file_id);

    Some(AssignmentFact {
        operator,
        target: assignment_target(target_element.as_ref(), target_span, file_id),
        target_span,
        value: assignment_value(value_element.clone(), file_id),
        value_span,
    })
}

fn assignment_target(
    element: Option<&SyntaxElement>,
    fallback_span: Span,
    file_id: FileId,
) -> AssignmentTarget {
    let Some(element) = element else {
        return AssignmentTarget::Compound {
            span: fallback_span,
        };
    };
    let span = span_for_element(element, file_id);
    let SyntaxElement::Token(token) = element else {
        return AssignmentTarget::Compound { span };
    };
    // A token target that cannot spell a name -- a bare `NULL`, a number, an
    // unterminated string -- is invalid rather than compound. Compound is for
    // node targets, which bind through a replacement function.
    let Some(delimiter) = name_delimiter(token) else {
        return AssignmentTarget::Invalid { span };
    };
    match RName::decode(token.text(), delimiter) {
        Ok(canonical) => AssignmentTarget::Binding(BindingName {
            canonical,
            spelling: span,
        }),
        // R rejects a zero-length variable name outright. That is invalid
        // source, not a name a later decoder could recover.
        Err(RNameDecodeError::EmptyName) => AssignmentTarget::Invalid { span },
        // Listed rather than bound with a catch-all so that a new variant
        // fails to compile here and has to be classified deliberately.
        Err(
            reason @ (RNameDecodeError::ContainsBackslash
            | RNameDecodeError::InvalidSpelling
            | RNameDecodeError::MixedUnicodeAndByteEscapes
            | RNameDecodeError::NulCharacter),
        ) => AssignmentTarget::Undecodable { span, reason },
    }
}
fn assignment_value(element: SyntaxElement, file_id: FileId) -> AssignmentValue {
    match Expr::cast(element) {
        Some(Expr::Function(function)) => {
            AssignmentValue::Function(function_fact(&function, file_id))
        }
        Some(Expr::Call(call)) => {
            let span = span_for_node(call.syntax(), file_id);
            AssignmentValue::Call(call_fact(call, file_id, span))
        }
        Some(Expr::Paren(paren)) => paren
            .inner()
            .map(|inner| assignment_value(inner, file_id))
            .unwrap_or(AssignmentValue::Other),
        Some(Expr::Name(identifier)) => {
            if identifier.is_reserved_constant() {
                AssignmentValue::Literal
            } else {
                let token = identifier.syntax();
                let value = name_delimiter(token)
                    .map_or(Err(RNameDecodeError::InvalidSpelling), |delimiter| {
                        RName::decode(token.text(), delimiter)
                    });
                AssignmentValue::Name(Spanned::new(value, span_for_token(token, file_id)))
            }
        }
        Some(
            Expr::IntLiteral(_)
            | Expr::FloatLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::ComplexLiteral(_),
        ) => AssignmentValue::Literal,
        _ => AssignmentValue::Other,
    }
}

fn call_fact(call: CallExpr, file_id: FileId, span: Span) -> CallFact {
    let callee = call_callee(&call, file_id);
    let arguments = call
        .args()
        .map(|argument| call_argument(argument, file_id))
        .collect::<Vec<_>>();
    let s7_class = s7_class_analysis(span, &callee, &arguments);
    CallFact {
        span,
        callee,
        arguments,
        s7_class,
    }
}

fn call_argument(argument: Arg, file_id: FileId) -> CallArgument {
    let name = argument.name_token().map(|token| {
        let span = span_for_token(&token, file_id);
        let value = name_delimiter(&token)
            .map_or(Err(RNameDecodeError::InvalidSpelling), |delimiter| {
                RName::decode(token.text(), delimiter)
            });
        Spanned::new(value, span)
    });
    let value = argument
        .value()
        .map(|element| call_argument_value(element, file_id))
        .unwrap_or(CallArgumentValue::Other);
    let value_span = argument
        .value()
        .map(|element| span_for_element(&element, file_id));
    CallArgument {
        name,
        value,
        value_span,
    }
}

fn call_argument_value(element: SyntaxElement, file_id: FileId) -> CallArgumentValue {
    match Expr::cast(element) {
        Some(Expr::Function(function)) => {
            CallArgumentValue::Function(function_fact(&function, file_id))
        }
        Some(Expr::Paren(paren)) => paren
            .inner()
            .map(|inner| call_argument_value(inner, file_id))
            .unwrap_or(CallArgumentValue::Other),
        Some(Expr::StringLiteral(literal)) => CallArgumentValue::String(Spanned::new(
            decode_string_literal(&literal),
            span_for_token(literal.syntax(), file_id),
        )),
        Some(Expr::Name(identifier)) if !identifier.is_reserved_constant() => {
            let token = identifier.syntax();
            let value = name_delimiter(token)
                .map_or(Err(RNameDecodeError::InvalidSpelling), |delimiter| {
                    RName::decode(token.text(), delimiter)
                });
            CallArgumentValue::Name(Spanned::new(value, span_for_token(token, file_id)))
        }
        Some(Expr::Name(identifier)) if identifier.constant() == Some(RConstant::Null) => {
            CallArgumentValue::Null
        }
        Some(
            Expr::Name(_) | Expr::IntLiteral(_) | Expr::FloatLiteral(_) | Expr::ComplexLiteral(_),
        ) => CallArgumentValue::Literal,
        _ => CallArgumentValue::Other,
    }
}

fn s7_class_analysis(
    call_span: Span,
    callee: &Option<Spanned<Result<CallCallee, RNameDecodeError>>>,
    arguments: &[CallArgument],
) -> S7ClassAnalysis {
    let is_new_class = matches!(
        callee.as_ref().and_then(|value| value.value.as_ref().ok()),
        Some(CallCallee::Simple(name)) if name.as_str() == "new_class"
    ) || matches!(
        callee.as_ref().and_then(|value| value.value.as_ref().ok()),
        Some(CallCallee::Namespace { package, name, internal: false })
            if package.as_str() == "S7" && name.as_str() == "new_class"
    );
    if !is_new_class {
        return S7ClassAnalysis::NotApplicable;
    }
    let Some(class_name) = arguments.first().and_then(|argument| {
        if argument.name.is_some() {
            return None;
        }
        match &argument.value {
            CallArgumentValue::String(Spanned {
                value: Ok(name),
                span,
            }) => Some(Spanned::new(name.clone(), *span)),
            _ => None,
        }
    }) else {
        let span = arguments
            .first()
            .and_then(|argument| argument.value_span)
            .unwrap_or(call_span);
        return S7ClassAnalysis::Refused(S7ClassRefusal {
            span,
            reason: S7ClassRefusalReason::ComputedClassName,
        });
    };
    let Some(constructor_argument) = arguments.iter().find(|argument| {
        argument
            .name
            .as_ref()
            .and_then(|name| name.value.as_ref().ok())
            .is_some_and(|name| name.as_str() == "constructor")
    }) else {
        return S7ClassAnalysis::Refused(S7ClassRefusal {
            span: call_span,
            reason: S7ClassRefusalReason::MissingConstructor,
        });
    };
    let CallArgumentValue::Function(constructor) = &constructor_argument.value else {
        return S7ClassAnalysis::Refused(S7ClassRefusal {
            span: constructor_argument.value_span.unwrap_or(call_span),
            reason: S7ClassRefusalReason::ComputedConstructor,
        });
    };
    S7ClassAnalysis::Supported(S7ClassFact {
        class_name,
        constructor: constructor.clone(),
    })
}

fn call_callee(
    call: &CallExpr,
    file_id: FileId,
) -> Option<Spanned<Result<CallCallee, RNameDecodeError>>> {
    if let Some(SyntaxElement::Node(node)) = call.base()
        && let Some(namespace) =
            BinaryExpr::cast(node.clone()).and_then(|binary| binary.namespace_access())
    {
        let value = namespace_callee(&namespace);
        return Some(Spanned::new(value, span_for_node(&node, file_id)));
    }

    let token = call.callee_token()?;
    let delimiter = name_delimiter(&token)?;
    let value = RName::decode(token.text(), delimiter).map(CallCallee::Simple);
    let span = span_for_token(&token, file_id);
    Some(Spanned::new(value, span))
}

fn namespace_callee(
    namespace: &arity_parser::ast::NamespaceAccess,
) -> Result<CallCallee, RNameDecodeError> {
    let package = RName::decode(
        namespace.package_token.text(),
        name_delimiter(&namespace.package_token).ok_or(RNameDecodeError::InvalidSpelling)?,
    )?;
    let name = RName::decode(
        namespace.name_token.text(),
        name_delimiter(&namespace.name_token).ok_or(RNameDecodeError::InvalidSpelling)?,
    )?;
    Ok(CallCallee::Namespace {
        package,
        name,
        internal: namespace.internal,
    })
}

#[cfg(test)]
mod tests {
    use crate::arity_adapter::test_support::{assignment, parsed, value_variant};
    use crate::arity_adapter::{
        AssignmentOperator, AssignmentTarget, AssignmentValue, CallCallee, S7ClassAnalysis,
        S7ClassRefusalReason, TopLevelShape,
    };
    #[test]
    fn canonicalizes_all_supported_binding_spellings() {
        let cases = [
            ("foo <- function(x) x", "foo", "foo"),
            ("`foo bar` <- function(x) x", "foo bar", "`foo bar`"),
            ("\"foo\" <- function(x) x", "foo", "\"foo\""),
            (
                "`[.myclass` <- function(x, i) x",
                "[.myclass",
                "`[.myclass`",
            ),
            ("`foo<-` <- function(x, value) x", "foo<-", "`foo<-`"),
            ("`%+%` <- function(a, b) a", "%+%", "`%+%`"),
            ("foo = function(x) x", "foo", "foo"),
            // Parentheses are required here: without them R parses the arrow
            // as part of the function body rather than as a top-level
            // assignment.
            ("(function(x) x) -> foo", "foo", "foo"),
            ("foo <<- function(x) x", "foo", "foo"),
        ];
        for (source_text, canonical, spelling) in cases {
            let (parsed, source) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentTarget::Binding(target) = &fact.target else {
                panic!("expected simple target for {source_text}");
            };
            // These values are the names R 4.x binds from the source spellings.
            assert_eq!(target.canonical.as_str(), canonical, "{source_text}");
            assert_eq!(source.text_range(target.spelling.range), Some(spelling));
        }
    }

    #[test]
    fn recognizes_every_assignment_operator_and_target_side() {
        let cases = [
            ("left <- 1", AssignmentOperator::Left, "left"),
            ("equals = 1", AssignmentOperator::Equals, "equals"),
            ("super <<- 1", AssignmentOperator::SuperLeft, "super"),
            ("1 -> right", AssignmentOperator::Right, "right"),
            (
                "1 ->> super_right",
                AssignmentOperator::SuperRight,
                "super_right",
            ),
            ("walrus := 1", AssignmentOperator::Walrus, "walrus"),
        ];
        for (source_text, operator, canonical) in cases {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert_eq!(fact.operator, operator);
            let AssignmentTarget::Binding(target) = &fact.target else {
                panic!("expected simple target for {source_text}");
            };
            // These values are the names R 4.x binds from each assignment form.
            assert_eq!(target.canonical.as_str(), canonical);
        }
    }

    #[test]
    fn leaves_complex_targets_without_a_binding() {
        for source_text in ["x[[1]] <- 1", "x$y <- 1", "dim(x) <- 1"] {
            let (parsed, source) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert!(
                matches!(fact.target, AssignmentTarget::Compound { .. }),
                "{source_text}"
            );
            assert_eq!(
                source.text_range(fact.target_span.range),
                Some(source_text.split(" <- ").next().unwrap())
            );
        }
    }

    #[test]
    fn classifies_assignment_values_and_shorthand_lambdas() {
        for (source_text, expected) in [
            ("f <- function(x) x", "function"),
            ("f <- \\(x) x", "function"),
            ("f <- g()", "call"),
            ("f <- g", "name"),
            ("f <- 1", "literal"),
            ("f <- if (x) 1 else 2", "other"),
        ] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert_eq!(value_variant(&fact.value), expected, "{source_text}");
        }
    }

    #[test]
    fn classifies_bare_top_level_atoms_and_calls() {
        let (parsed, source) = parsed(
            r#"NULL
TRUE
"value"
foo()
"#,
        );
        assert!(matches!(
            parsed.top_level[0].fact.shape,
            TopLevelShape::Null
        ));
        assert!(matches!(
            parsed.top_level[1].fact.shape,
            TopLevelShape::Other
        ));
        let TopLevelShape::StringLiteral(value) = &parsed.top_level[2].fact.shape else {
            panic!("expected string literal");
        };
        assert_eq!(value.value.as_ref().unwrap().as_str(), "value");
        assert_eq!(source.text_range(value.span.range), Some("\"value\""));
        let TopLevelShape::Call(call) = &parsed.top_level[3].fact.shape else {
            panic!("expected call");
        };
        let Some(Ok(CallCallee::Simple(name))) = call.callee.as_ref().map(|callee| &callee.value)
        else {
            panic!("expected a decoded simple callee");
        };
        assert_eq!(name.as_str(), "foo");
        assert_eq!(source.text_range(call.span.range), Some("foo()"));
    }

    #[test]
    fn extracts_supported_s7_class_constructor_metadata() {
        let (parsed, _) = parsed(
            r#"Foo <- S7::new_class(
  "Foo",
  properties = list(),
  constructor = function(..., value = list()) NULL
)
"#,
        );
        let TopLevelShape::Assignment(assignment) = &parsed.top_level[0].fact.shape else {
            panic!("expected assignment");
        };
        let AssignmentValue::Call(call) = &assignment.value else {
            panic!("expected call assignment");
        };
        let S7ClassAnalysis::Supported(class) = &call.s7_class else {
            panic!("expected supported S7 metadata");
        };
        assert_eq!(class.class_name.value, "Foo");
        let formals = class
            .constructor
            .formals
            .as_ref()
            .expect("constructor formals");
        assert_eq!(formals.len(), 2);
        assert_eq!(formals[0].name.value.as_ref().unwrap().as_str(), "...");
        assert_eq!(formals[1].name.value.as_ref().unwrap().as_str(), "value");
        assert!(formals[1].default.is_some());
    }

    #[test]
    fn extracts_unqualified_s7_class_constructor_metadata() {
        let (parsed, _) = parsed(
            r#"Foo <- new_class("Foo", constructor = function(value = 1) NULL)
"#,
        );
        let TopLevelShape::Assignment(assignment) = &parsed.top_level[0].fact.shape else {
            panic!("expected assignment");
        };
        let AssignmentValue::Call(call) = &assignment.value else {
            panic!("expected call assignment");
        };
        assert!(matches!(call.s7_class, S7ClassAnalysis::Supported(_)));
    }

    #[test]
    fn refuses_computed_or_missing_s7_constructor_metadata() {
        let cases = [
            (
                r#"Foo <- new_class(class_name, constructor = make_constructor())
"#,
                S7ClassRefusalReason::ComputedClassName,
            ),
            (
                r#"Foo <- new_class("Foo")
"#,
                S7ClassRefusalReason::MissingConstructor,
            ),
            (
                r#"Foo <- new_class("Foo", properties = list(make = function(x) x))
"#,
                S7ClassRefusalReason::MissingConstructor,
            ),
            (
                r#"Foo <- new_class("Foo", constructor = make_constructor())
"#,
                S7ClassRefusalReason::ComputedConstructor,
            ),
        ];
        for (source, reason) in cases {
            let (parsed, _) = parsed(source);
            let TopLevelShape::Assignment(assignment) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment");
            };
            assert!(matches!(
                assignment.value,
                AssignmentValue::Call(ref call)
                    if matches!(call.s7_class, S7ClassAnalysis::Refused(ref refusal) if refusal.reason == reason)
            ));
        }
    }

    #[test]
    fn does_not_treat_internal_s7_new_class_as_public_constructor() {
        let (parsed, _) = parsed(
            r#"Foo <- S7:::new_class("Foo", constructor = function() NULL)
"#,
        );
        let TopLevelShape::Assignment(assignment) = &parsed.top_level[0].fact.shape else {
            panic!("expected assignment");
        };
        let AssignmentValue::Call(call) = &assignment.value else {
            panic!("expected call assignment");
        };
        assert!(matches!(call.s7_class, S7ClassAnalysis::NotApplicable));
    }

    #[test]
    fn records_qualified_callee_and_source_ordered_spans() {
        let text = r#"first <- 1; pkg::second()
"third"
"#;
        let (parsed, source) = parsed(text);
        assert_eq!(parsed.top_level.len(), 3);
        assert_eq!(
            source.text_range(parsed.top_level[0].fact.span.range),
            Some("first <- 1")
        );
        assert_eq!(
            source.text_range(parsed.top_level[1].fact.span.range),
            Some("pkg::second()")
        );
        assert_eq!(
            source.text_range(parsed.top_level[2].fact.span.range),
            Some("\"third\"")
        );
        let TopLevelShape::Call(call) = &parsed.top_level[1].fact.shape else {
            panic!("expected call");
        };
        let Some(Ok(CallCallee::Namespace {
            package,
            name,
            internal: false,
        })) = call.callee.as_ref().map(|callee| &callee.value)
        else {
            panic!("expected a decoded namespace callee");
        };
        assert_eq!(package.as_str(), "pkg");
        assert_eq!(name.as_str(), "second");
        assert_eq!(
            source.text_range(call.callee.as_ref().unwrap().span.range),
            Some("pkg::second")
        );
    }

    #[test]
    fn syntax_diagnostics_fail_closed_for_facts() {
        let (parsed, _) = parsed(
            r#"broken <-
"#,
        );
        assert!(!parsed.diagnostics.is_empty());
        assert!(parsed.top_level.is_empty());
    }

    #[test]
    fn assignment_spans_point_at_target_and_value_text() {
        let text = r#"f <- function(x) x
"#;
        let (parsed, source) = parsed(text);
        let TopLevelShape::Assignment(fact) = &assignment(&parsed, 0).shape else {
            panic!("expected assignment");
        };
        assert_eq!(source.text_range(fact.target_span.range), Some("f"));
        assert_eq!(
            source.text_range(fact.value_span.range),
            Some("function(x) x")
        );
    }

    #[test]
    fn classifies_reserved_constants_but_not_rebindable_boolean_symbols() {
        for (source_text, expected) in [
            ("x <- NULL", "literal"),
            ("x <- TRUE", "literal"),
            ("x <- NA", "literal"),
            ("x <- Inf", "literal"),
            ("x <- T", "name"),
            ("x <- F", "name"),
            ("x <- y", "name"),
        ] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert_eq!(value_variant(&fact.value), expected, "{source_text}");
        }
    }

    #[test]
    fn sees_through_parentheses_recursively_for_assignment_values() {
        for (source_text, expected) in [
            ("(function(x) x) -> foo", "function"),
            ("x <- (name)", "name"),
            ("x <- (NULL)", "literal"),
            ("x <- (call())", "call"),
            ("x <- ((NULL))", "literal"),
        ] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert_eq!(value_variant(&fact.value), expected, "{source_text}");
        }
    }

    #[test]
    fn decodes_assignment_value_names_and_preserves_their_spans() {
        for (source_text, expected_name, expected_spelling) in
            [("b <- a", "a", "a"), ("b <- `a b`", "a b", "`a b`")]
        {
            let (parsed, source) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentValue::Name(name) = &fact.value else {
                panic!("expected a decoded assignment name for {source_text}");
            };
            assert_eq!(name.value.as_ref().unwrap().as_str(), expected_name);
            assert_eq!(source.text_range(name.span.range), Some(expected_spelling));
        }
    }

    #[test]
    fn decodes_each_component_of_a_namespace_callee() {
        let (parsed, _) = parsed(
            r#""pkg"::r"(name)"()
"#,
        );
        let TopLevelShape::Call(call) = &parsed.top_level[0].fact.shape else {
            panic!("expected call");
        };
        let Some(Ok(CallCallee::Namespace {
            package,
            name,
            internal: false,
        })) = call.callee.as_ref().map(|callee| &callee.value)
        else {
            panic!("expected decoded namespace callee");
        };
        assert_eq!(package.as_str(), "pkg");
        assert_eq!(name.as_str(), "name");
    }
}
