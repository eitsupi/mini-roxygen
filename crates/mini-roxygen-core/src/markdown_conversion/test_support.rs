//! Shared fixtures for Markdown conversion tests.

use std::path::PathBuf;

use rd_ast::{RdDocument, RdNode};
use rd_writer::Writer;

use super::{HelpLinkResolver, LinkResolution, MarkdownContext};
use crate::rd_oracle::{assert_r_accepts, minimal_topic};
use crate::source::{FileId, SourceFile, Span, TextRange};
use crate::tags::{MarkdownText, NormalizeHead, SourcedText};

struct EmptyResolver;

impl HelpLinkResolver for EmptyResolver {
    fn resolve_unqualified(&self, _topic: &str) -> LinkResolution {
        LinkResolution::Local
    }
}

static EMPTY_RESOLVER: EmptyResolver = EmptyResolver;

pub(crate) fn context() -> MarkdownContext<'static> {
    MarkdownContext {
        current_package: None,
        links: &EMPTY_RESOLVER,
        inline_r_session: None,
    }
}

pub(crate) fn value(text: &str) -> MarkdownText {
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let end = u32::try_from(source.text().len()).expect("test text length fits u32");
    MarkdownText::new(SourcedText::from_lines(
        &source,
        &[Span::new(FileId::new(0), TextRange::new(0, end))],
        NormalizeHead::Intro,
    ))
}

pub(crate) fn serialize(nodes: Vec<RdNode>) -> String {
    Writer::new(rd_writer::WriterOptions::default())
        .write_document(&RdDocument::from(nodes))
        .expect("writer accepts the converted fragment")
}

pub(crate) fn assert_serialized_body(nodes: Vec<RdNode>, expected: &str) {
    let body = serialize(nodes);
    assert_eq!(body, expected);
    assert_r_accepts(&minimal_topic(&body));
}
