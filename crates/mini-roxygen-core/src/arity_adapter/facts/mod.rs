//! Coordinates the parser-independent fact modules.
//!
//! The responsibilities are kept in separate modules so name decoding, function shape, and top-level classification can evolve independently while sharing one provenance implementation.

mod function;
mod name;
mod spans;
mod top_level;

use function::function_fact;
use name::{RNameDelimiter, decode_string_literal, name_delimiter};
use spans::{
    span_for_element, span_for_expression, span_for_node, span_for_offsets, span_for_token,
};

#[cfg(test)]
pub(crate) use function::FunctionFact;
pub use function::{Formal, FormalError};
pub(super) use name::decode_authors_string_literal;
pub use name::{RName, RNameDecodeError};
pub use top_level::{
    AssignmentFact, AssignmentOperator, AssignmentTarget, AssignmentValue, BindingName,
    CallArgument, CallArgumentValue, CallCallee, CallFact, S7ClassAnalysis, S7ClassFact,
    S7ClassRefusal, S7ClassRefusalReason, TopLevelFact, TopLevelShape,
};
pub(super) use top_level::{nested_call_facts, top_level_facts};
