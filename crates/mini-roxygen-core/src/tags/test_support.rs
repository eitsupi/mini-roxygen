//! Shared helpers for semantic tag unit tests.

use std::path::PathBuf;

use super::{ParsedTag, TagParseOptions, UnknownTagPolicy, parse_block, split_section_title};
use crate::arity_adapter::parse;
use crate::diagnostic::Diagnostics;
use crate::source::{FileId, SourceFile};

pub(crate) fn parsed(
    text: &str,
    policy: UnknownTagPolicy,
) -> (Vec<ParsedTag>, Diagnostics, SourceFile) {
    let source = SourceFile::new(
        PathBuf::from("test.R"),
        format!(
            r#"{text}
NULL
"#
        ),
    );
    let parsed = parse(&source, FileId::new(0));
    let options = TagParseOptions {
        unknown_tags: policy,
    };
    let block = parsed.top_level[0]
        .documentation
        .as_ref()
        .expect("expected documentation");
    let (tags, diagnostics) = parse_block(&source, block, &options);
    (tags, diagnostics, source)
}

pub(crate) fn split_parts(value: &str) -> (&str, &str) {
    let (title_end, body_start) = split_section_title(value).expect("section separator");
    (&value[..title_end], &value[body_start..])
}
