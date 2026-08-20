//! Shared fixtures for model unit tests.
//!
//! These builders stay in one test-only module because every responsibility
//! tests the same source-to-block setup and duplicating it would obscure the behavior under test.

use std::path::PathBuf;

use crate::arity_adapter::parse;
use crate::diagnostic::Diagnostics;
use crate::r_parse::build_object_index;
use crate::source::{SourceFile, SourceMap};
use crate::tags::{TagParseOptions, UnknownTagPolicy};

use super::{
    BlockRef, DocumentedBlock, ModelOutput, build_package_model, build_package_model_with_bindings,
};

pub(crate) fn blocks(source_map: &mut SourceMap, path: &str, text: &str) -> Vec<DocumentedBlock> {
    let source = SourceFile::new(PathBuf::from(path), text.to_owned());
    let file = source_map.add_file(source.clone());
    let index = build_object_index(parse(&source, file), file);
    let parsed = parse(&source, file);
    index
        .documented
        .into_iter()
        .map(|object| {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("object index entries have documentation");
            let (tags, tag_diagnostics) = crate::tags::parse_block(
                &source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
            );
            assert!(tag_diagnostics.is_empty(), "valid test tags parse");
            DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            }
        })
        .collect()
}

pub(crate) fn model(text: &str) -> ModelOutput {
    let mut source_map = SourceMap::new();
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let file = source_map.add_file(source.clone());
    let parsed = parse(&source, file);
    let index = build_object_index(parsed, file);
    let bindings = index.bindings.clone();
    let parsed = parse(&source, file);
    let blocks = index
        .documented
        .into_iter()
        .map(|object| {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("object index entries have documentation");
            let (tags, tag_diagnostics) = crate::tags::parse_block(
                &source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
            );
            assert!(tag_diagnostics.is_empty(), "valid test tags parse");
            DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            }
        })
        .collect();
    build_package_model_with_bindings(&source_map, blocks, bindings)
}

/// Builds a source fixture while retaining tag-parser diagnostics.
pub(crate) fn model_with_tag_diagnostics(text: &str) -> (ModelOutput, Diagnostics) {
    let mut source_map = SourceMap::new();
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let file = source_map.add_file(source.clone());
    let parsed = parse(&source, file);
    let index = build_object_index(parsed, file);
    let parsed = parse(&source, file);
    let mut diagnostics = Diagnostics::new();
    let blocks = index
        .documented
        .into_iter()
        .map(|object| {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("object index entries have documentation");
            let (tags, tag_diagnostics) = crate::tags::parse_block(
                &source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
            );
            for diagnostic in tag_diagnostics.iter().cloned() {
                diagnostics.push(diagnostic);
            }
            DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            }
        })
        .collect();
    (build_package_model(&source_map, blocks), diagnostics)
}

/// Builds a model while retaining both tag diagnostics and its source map.
///
/// This variant is used by end-to-end tests that must keep malformed tags out
/// of the semantic model but still lower the surviving contributions.
pub(crate) fn model_with_tag_diagnostics_and_sources(
    text: &str,
) -> (ModelOutput, Diagnostics, SourceMap) {
    let mut source_map = SourceMap::new();
    let source = SourceFile::new(PathBuf::from("test.R"), text.to_owned());
    let file = source_map.add_file(source.clone());
    let parsed = parse(&source, file);
    let index = build_object_index(parsed, file);
    let parsed = parse(&source, file);
    let mut diagnostics = Diagnostics::new();
    let blocks = index
        .documented
        .into_iter()
        .map(|object| {
            let raw = parsed
                .top_level
                .iter()
                .find_map(|entry| {
                    entry
                        .documentation
                        .as_ref()
                        .filter(|block| block.id == object.block)
                })
                .expect("object index entries have documentation");
            let (tags, tag_diagnostics) = crate::tags::parse_block(
                &source,
                raw,
                &TagParseOptions::default().with_unknown_tags(UnknownTagPolicy::Ignore),
            );
            for diagnostic in tag_diagnostics.iter().cloned() {
                diagnostics.push(diagnostic);
            }
            DocumentedBlock {
                block: BlockRef {
                    file,
                    block: object.block,
                },
                block_span: object.block_span,
                target: object.target,
                tags,
            }
        })
        .collect();
    (
        build_package_model(&source_map, blocks),
        diagnostics,
        source_map,
    )
}
