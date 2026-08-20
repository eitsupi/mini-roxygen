//! Provides the small tag queries used while assembling topics.
//!
//! Keeping first-value and policy lookups together prevents merge ordering
//! from being coupled to the details of individual tag representations.

use crate::tags::{DefaultAliasPolicy, ParsedTag, PlainText, TagValue};

pub(in crate::model) fn first_name(tags: &[ParsedTag]) -> Option<&TagValue<PlainText>> {
    tags.iter().find_map(|tag| match tag {
        ParsedTag::Name(value) => Some(value),
        _ => None,
    })
}

pub(in crate::model) fn first_rdname(tags: &[ParsedTag]) -> Option<&TagValue<PlainText>> {
    tags.iter().find_map(|tag| match tag {
        ParsedTag::RdName(value) => Some(value),
        _ => None,
    })
}

pub(in crate::model) fn first_order(tags: &[ParsedTag]) -> Option<i64> {
    tags.iter().find_map(|tag| match tag {
        ParsedTag::Order { value, .. } => Some(*value),
        _ => None,
    })
}

pub(in crate::model) fn has_no_rd(tags: &[ParsedTag]) -> bool {
    tags.iter().any(|tag| matches!(tag, ParsedTag::NoRd(_)))
}

pub(in crate::model) fn suppresses_default_aliases(tags: &[ParsedTag]) -> bool {
    tags.iter().any(|tag| {
        matches!(tag, ParsedTag::Aliases(directive)
            if directive.value.defaults == DefaultAliasPolicy::Suppress)
    })
}
