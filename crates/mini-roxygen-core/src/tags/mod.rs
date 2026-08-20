//! Converts raw roxygen tags into the first semantic tag layer.
//!
//! The registry assigns structured meanings to recognized tags without losing
//! source text or provenance.

mod diagnostics;
mod inherit;
mod intro;
mod model;
mod param;
mod registry;
mod section;
#[cfg(test)]
pub(crate) mod test_support;
mod text;
mod words;

pub use model::{
    AliasDirective, ArgSelection, ArgSelector, DefaultAliasPolicy, DocName, ExamplesContent,
    ExamplesIf, FieldTag, FieldValue, InheritField, InheritFields, InheritTarget, Keyword,
    MarkdownText, NamespaceTag, ParamName, ParsedTag, PlainText, RCodeText, TagOrigin,
    TagParseOptions, TagValue, TopicRef, UnknownTag, UnknownTagPolicy, UnsupportedTag,
    UsageDirective,
};
use registry::NamespaceTagKind;
pub(crate) use section::split_section_title;
pub(crate) use text::NormalizeHead;
pub use text::SourcedText;

use crate::arity_adapter::{RawTag, RoxyBlock, can_parse_r};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::{SourceFile, TextRange};

use self::diagnostics::{
    emit_tag_diagnostic, emit_unknown_diagnostic, emit_unsupported_diagnostic, value_span,
    value_span_for_range,
};
use self::inherit::{parse_inherit, parse_inherit_params, parse_inherit_section};
use self::param::parse_param;
use self::registry::{KnownTagKind, Multiline, TagSpec, ValueRequirement, tag_spec};
use self::section::emit_section_diagnostic;
use self::words::parse_words;

/// Parses the explicit tags in one raw roxygen block.
///
/// Intro text is decomposed after explicit tags are parsed, so raw tag names
/// can control the implicit title/description/details reconciliation.
#[must_use]
pub fn parse_block(
    source_file: &SourceFile,
    block: &RoxyBlock,
    options: &TagParseOptions,
) -> (Vec<ParsedTag>, Diagnostics) {
    let mut parsed = Vec::with_capacity(block.tags.len());
    let mut diagnostics = Diagnostics::new();

    for (raw_index, raw_tag) in block.tags.iter().enumerate() {
        let normalized =
            SourcedText::from_lines(source_file, &raw_tag.value_lines, NormalizeHead::TagValue);
        let trimmed = trim_outer(normalized.clone());
        let origin = TagOrigin::Explicit {
            name: raw_tag.name.clone(),
            value_span: raw_tag.value_span,
            full_span: raw_tag.full_span,
        };

        let Some(spec) = tag_spec(raw_tag.name.value.as_str()) else {
            let unknown = UnknownTag {
                name: raw_tag.name.clone(),
                value: normalized,
                value_span: raw_tag.value_span,
                full_span: raw_tag.full_span,
            };
            emit_unknown_diagnostic(&mut diagnostics, raw_tag, options.unknown_tags);
            parsed.push(intro::ParsedTagEntry {
                raw_index,
                tag: ParsedTag::Unknown(unknown),
            });
            continue;
        };

        // Param-like parsers retain their deliberately more specific empty-value
        // diagnostics. Every other requirement is checked before its grammar.
        if !matches!(
            spec.kind,
            KnownTagKind::Param
                | KnownTagKind::ExamplesIf
                | KnownTagKind::Inherit
                | KnownTagKind::InheritSection
                | KnownTagKind::InheritParams
        ) && !check_value_requirement(
            raw_tag,
            &normalized,
            &trimmed,
            spec.requirement,
            &mut diagnostics,
        ) {
            continue;
        }

        let semantic_value = match spec.kind {
            KnownTagKind::Namespace(NamespaceTagKind::RawNamespace) => normalized,
            KnownTagKind::Examples => remove_one_leading_newline(normalized),
            _ => trimmed,
        };
        let semantic_value =
            apply_multiline(raw_tag, semantic_value, spec.multiline, &mut diagnostics);

        let section_separator = matches!(spec.kind, KnownTagKind::Section)
            .then(|| split_section_title(semantic_value.as_str()))
            .flatten();
        let malformed_section =
            matches!(spec.kind, KnownTagKind::Section) && section_separator.is_none();
        let multiline_malformed_section =
            malformed_section && semantic_value.as_str().contains('\n');
        if malformed_section {
            emit_section_diagnostic(
                &mut diagnostics,
                raw_tag,
                &semantic_value,
                multiline_malformed_section,
            );
        }
        if multiline_malformed_section {
            continue;
        }

        if let Some(tag) = parse_tag(
            raw_tag,
            spec,
            semantic_value,
            section_separator,
            origin,
            &mut diagnostics,
        ) {
            parsed.push(intro::ParsedTagEntry { raw_index, tag });
        }
    }

    (
        intro::reconcile(source_file, block.intro.as_ref(), &block.tags, parsed),
        diagnostics,
    )
}

fn parse_tag(
    raw_tag: &RawTag,
    spec: TagSpec,
    value: SourcedText,
    section_separator: Option<(usize, usize)>,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let tag = match spec.kind {
        KnownTagKind::Title => ParsedTag::Title(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Description => {
            ParsedTag::Description(parse_field(value, origin, MarkdownText::new))
        }
        KnownTagKind::Details => ParsedTag::Details(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Return => ParsedTag::Return(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::SeeAlso => ParsedTag::SeeAlso(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::References => {
            ParsedTag::References(parse_field(value, origin, MarkdownText::new))
        }
        KnownTagKind::Note => ParsedTag::Note(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Format => ParsedTag::Format(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Source => ParsedTag::Source(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Author => ParsedTag::Author(parse_field(value, origin, MarkdownText::new)),
        KnownTagKind::Param => parse_param(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::Name => ParsedTag::Name(TagValue {
            value: PlainText::new(value.clone()),
            origin,
        }),
        KnownTagKind::RdName => ParsedTag::RdName(TagValue {
            value: PlainText::new(value.clone()),
            origin,
        }),
        KnownTagKind::Aliases => ParsedTag::Aliases(TagValue {
            value: parse_alias_directive(&value),
            origin,
        }),
        KnownTagKind::Keywords => ParsedTag::Keywords(parse_field(value, origin, |value| {
            parse_words(&value, |word| Keyword(word.to_owned()))
        })),
        KnownTagKind::NoRd => ParsedTag::NoRd(origin),
        KnownTagKind::Examples => ParsedTag::Examples(TagValue {
            value: ExamplesContent::Ordinary(RCodeText::new(value)),
            origin,
        }),
        KnownTagKind::ExamplesIf => {
            let newline = value.as_str().find('\n');
            let condition_end = newline.unwrap_or(value.as_str().len());
            let condition = value.slice(TextRange::new(
                0,
                u32::try_from(condition_end).expect("normalized text length fits u32"),
            ));
            let body_start = newline.map_or(condition_end, |offset| offset + 1);
            let body = value.slice(TextRange::new(
                u32::try_from(body_start).expect("normalized text length fits u32"),
                u32::try_from(value.as_str().len()).expect("normalized text length fits u32"),
            ));
            if condition.as_str().trim().is_empty() || !can_parse_r(condition.as_str().trim()) {
                emit_tag_diagnostic(
                    diagnostics,
                    raw_tag,
                    DiagnosticCode::InvalidExamplesIfCondition,
                    "@examplesIf condition failed to parse",
                    value_span_for_range(&value, 0, condition_end, raw_tag.value_span),
                );
                return None;
            }
            if body.as_str().trim().is_empty() {
                emit_tag_diagnostic(
                    diagnostics,
                    raw_tag,
                    DiagnosticCode::EmptyExamplesIfBody,
                    "@examplesIf requires example code after the condition",
                    value_span_for_range(
                        &value,
                        body_start,
                        value.as_str().len(),
                        raw_tag.value_span,
                    ),
                );
                return None;
            }
            ParsedTag::Examples(TagValue {
                value: ExamplesContent::Conditional(ExamplesIf {
                    condition: RCodeText::new(condition),
                    body: RCodeText::new(body),
                }),
                origin,
            })
        }
        KnownTagKind::Usage => ParsedTag::Usage(TagValue {
            value: if value.as_str() == "NULL" {
                UsageDirective::SuppressGenerated
            } else {
                UsageDirective::Explicit(RCodeText::new(value))
            },
            origin,
        }),
        KnownTagKind::Method => parse_method(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::Order => parse_order(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::Inherit => parse_inherit(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::InheritSection => parse_inherit_section(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::InheritParams => parse_inherit_params(raw_tag, value, origin, diagnostics)?,
        KnownTagKind::Section => {
            let end = value.as_str().len();
            let (title_end, body_start) = section_separator.unwrap_or((end, end));
            ParsedTag::Section {
                title: MarkdownText::new(value.slice(TextRange::new(
                    0,
                    u32::try_from(title_end).expect("normalized text length fits u32"),
                ))),
                body: MarkdownText::new(value.slice(TextRange::new(
                    u32::try_from(body_start).expect("normalized text length fits u32"),
                    u32::try_from(end).expect("normalized text length fits u32"),
                ))),
                origin,
            }
        }
        KnownTagKind::Namespace(kind) => ParsedTag::Namespace(namespace_tag(
            kind,
            TagValue {
                value: PlainText::new(value),
                origin,
            },
        )),
        KnownTagKind::Unsupported(kind) => {
            emit_unsupported_diagnostic(diagnostics, raw_tag, &value);
            ParsedTag::Unsupported(UnsupportedTag {
                kind,
                value: TagValue {
                    value: PlainText::new(value),
                    origin,
                },
            })
        }
        KnownTagKind::MarkdownMarker | KnownTagKind::NoMd => return None,
    };
    Some(tag)
}

fn parse_field<T>(
    value: SourcedText,
    origin: TagOrigin,
    convert: impl FnOnce(SourcedText) -> T,
) -> TagValue<FieldValue<T>> {
    // The sentinel is classified before conversion because rendered Markdown
    // or split words cannot reliably preserve the exact raw scalar value.
    let field = if value.as_str() == "NULL" {
        FieldValue::Suppress
    } else {
        FieldValue::Emit(convert(value))
    };
    TagValue {
        value: field,
        origin,
    }
}

fn parse_alias_directive(value: &SourcedText) -> AliasDirective {
    let explicit = parse_words(value, |word| DocName(word.to_owned()))
        .into_iter()
        .filter(|word| word.value.0 != "NULL")
        .collect();
    let defaults = if value.as_str().split_whitespace().any(|word| word == "NULL") {
        DefaultAliasPolicy::Suppress
    } else {
        DefaultAliasPolicy::Include
    };
    AliasDirective { explicit, defaults }
}

fn namespace_tag(kind: NamespaceTagKind, value: TagValue<PlainText>) -> NamespaceTag {
    match kind {
        NamespaceTagKind::Export => NamespaceTag::Export(value),
        NamespaceTagKind::ExportS3Method => NamespaceTag::ExportS3Method(value),
        NamespaceTagKind::Import => NamespaceTag::Import(value),
        NamespaceTagKind::ImportFrom => NamespaceTag::ImportFrom(value),
        NamespaceTagKind::RawNamespace => NamespaceTag::RawNamespace(value),
        NamespaceTagKind::UseDynLib => NamespaceTag::UseDynLib(value),
        NamespaceTagKind::ExportPattern => NamespaceTag::ExportPattern(value),
        NamespaceTagKind::ExportClass => NamespaceTag::ExportClass(value),
        NamespaceTagKind::ExportMethod => NamespaceTag::ExportMethod(value),
        NamespaceTagKind::ImportClassesFrom => NamespaceTag::ImportClassesFrom(value),
        NamespaceTagKind::ImportMethodsFrom => NamespaceTag::ImportMethodsFrom(value),
    }
}

fn parse_method(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let words = crate::tags::words::word_ranges(&value);
    if words.len() != 2 {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            format!("@method requires exactly two words, not {}", words.len()),
            value_span(&value, raw_tag.value_span),
        );
        return None;
    }

    let (generic_start, generic_end) = words[0];
    let (class_start, class_end) = words[1];
    Some(ParsedTag::Method {
        generic: crate::source::Spanned::new(
            value.as_str()[generic_start..generic_end].to_owned(),
            value_span_for_range(&value, generic_start, generic_end, raw_tag.value_span),
        ),
        class: crate::source::Spanned::new(
            value.as_str()[class_start..class_end].to_owned(),
            value_span_for_range(&value, class_start, class_end, raw_tag.value_span),
        ),
        origin,
    })
}

fn parse_order(
    raw_tag: &RawTag,
    value: SourcedText,
    origin: TagOrigin,
    diagnostics: &mut Diagnostics,
) -> Option<ParsedTag> {
    let Ok(order) = value.as_str().parse::<i64>() else {
        emit_tag_diagnostic(
            diagnostics,
            raw_tag,
            DiagnosticCode::TagParseError,
            "@order requires a single integer",
            value_span(&value, raw_tag.value_span),
        );
        return None;
    };
    Some(ParsedTag::Order {
        value: order,
        origin,
    })
}

fn check_value_requirement(
    raw_tag: &RawTag,
    normalized: &SourcedText,
    trimmed: &SourcedText,
    requirement: ValueRequirement,
    diagnostics: &mut Diagnostics,
) -> bool {
    match requirement {
        ValueRequirement::Required if trimmed.is_empty() => {
            // These are errors rather than roxygen2's warnings because an empty
            // semantic value would otherwise leak invalid IR into later layers.
            let span = if normalized.is_empty() {
                raw_tag.value_span
            } else {
                value_span(normalized, raw_tag.value_span)
            };
            diagnostics.push(
                Diagnostic::new(
                    Severity::Error,
                    DiagnosticCode::TagParseError,
                    format!("@{} requires a value", raw_tag.name.value),
                    Label::new(span, format!("@{} is missing a value", raw_tag.name.value)),
                )
                .with_context("tag", raw_tag.name.value.clone()),
            );
            false
        }
        ValueRequirement::Forbidden if !trimmed.is_empty() => {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                format!("@{} must not be followed by any text", raw_tag.name.value),
                value_span(normalized, raw_tag.value_span),
            );
            false
        }
        _ => true,
    }
}

fn apply_multiline(
    raw_tag: &RawTag,
    value: SourcedText,
    policy: Multiline,
    diagnostics: &mut Diagnostics,
) -> SourcedText {
    let lines = value.as_str().split('\n').collect::<Vec<_>>();
    if lines.len() <= 1 {
        return value;
    }

    match policy {
        Multiline::Always => value,
        Multiline::Never => {
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                format!(
                    "@{} must be only 1 line long, not {}",
                    raw_tag.name.value,
                    lines.len()
                ),
                value_span(&value, raw_tag.value_span),
            );
            value
        }
        Multiline::Indent => {
            let first_indent = leading_spaces(lines[0]);
            let first_failure = lines.iter().skip(1).position(|line| {
                !line.chars().any(|character| !character.is_whitespace())
                    || leading_spaces(line) <= first_indent
            });
            let Some(relative_failure) = first_failure else {
                return value;
            };
            let failure_index = relative_failure + 1;
            let failure_start = lines[..failure_index]
                .iter()
                .map(|line| line.len())
                .sum::<usize>()
                + failure_index;
            let retained_end = failure_start.saturating_sub(1);
            let retained = value.slice(TextRange::new(
                0,
                u32::try_from(retained_end).expect("normalized text length fits u32"),
            ));
            let diagnostic_end = failure_start
                + lines[failure_index]
                    .chars()
                    .next()
                    .map_or(0, |character| character.len_utf8());
            let diagnostic_span = value_span_for_range(
                &value,
                failure_start,
                diagnostic_end.min(value.as_str().len()),
                raw_tag.value_span,
            );
            emit_tag_diagnostic(
                diagnostics,
                raw_tag,
                DiagnosticCode::TagParseError,
                format!(
                    "@{} continuation must use a hanging indent",
                    raw_tag.name.value
                ),
                diagnostic_span,
            );
            retained
        }
    }
}

fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn remove_one_leading_newline(value: SourcedText) -> SourcedText {
    if value.as_str().starts_with('\n') {
        value.slice(TextRange::new(
            1,
            u32::try_from(value.as_str().len()).expect("normalized text length fits u32"),
        ))
    } else {
        value
    }
}

pub(super) fn trim_outer(value: SourcedText) -> SourcedText {
    let trimmed = value.as_str().trim();
    let start = value.as_str().len() - value.as_str().trim_start().len();
    let end = start + trimmed.len();
    value.slice(TextRange::new(
        u32::try_from(start).expect("normalized text length fits u32"),
        u32::try_from(end).expect("normalized text length fits u32"),
    ))
}

#[cfg(test)]
mod tests {
    use super::{NamespaceTag, ParsedTag, UnknownTagPolicy};
    use crate::tags::test_support::parsed;

    #[test]
    fn parsed_tags_keep_source_order() {
        let (tags, _, _) = parsed(
            r"#' @future first
#' @title second
#' @other third
",
            UnknownTagPolicy::Ignore,
        );
        assert!(matches!(&tags[0], ParsedTag::Unknown(_)));
        assert!(matches!(&tags[1], ParsedTag::Title(_)));
        assert!(matches!(&tags[2], ParsedTag::Unknown(_)));
    }

    #[test]
    fn multiline_policies_preserve_or_truncate_as_specified() {
        let (never_tags, never_diagnostics, _) = parsed(
            r"#' @name topic
#' second line
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Name(name) = &never_tags[0] else {
            panic!("expected name");
        };
        assert_eq!(name.value.as_str(), "topic\nsecond line");
        assert_eq!(never_diagnostics.len(), 1);

        let (blank_tags, blank_diagnostics, _) = parsed(
            r"#' @importFrom pkg fun
#'   continuation
#'
#'   dropped
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Namespace(namespace) = &blank_tags[0] else {
            panic!("expected namespace tag");
        };
        let NamespaceTag::ImportFrom(value) = &namespace else {
            panic!("expected importFrom");
        };
        assert_eq!(value.value.as_str(), "pkg fun\n  continuation");
        assert_eq!(blank_diagnostics.len(), 1);
        assert!(
            blank_diagnostics
                .iter()
                .next()
                .expect("diagnostic")
                .primary
                .span
                .range
                .start()
                > 0
        );

        let (flush_tags, flush_diagnostics, _) = parsed(
            r"#' @importFrom pkg fun
#' flush
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Namespace(namespace) = &flush_tags[0] else {
            panic!("expected namespace tag");
        };
        let NamespaceTag::ImportFrom(value) = &namespace else {
            panic!("expected importFrom");
        };
        assert_eq!(value.value.as_str(), "pkg fun");
        assert_eq!(flush_diagnostics.len(), 1);

        let (tab_tags, tab_diagnostics, _) = parsed(
            "#' @importFrom pkg fun\n#'\tcontinuation\n",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Namespace(namespace) = &tab_tags[0] else {
            panic!("expected namespace tag");
        };
        let NamespaceTag::ImportFrom(value) = &namespace else {
            panic!("expected importFrom");
        };
        assert_eq!(value.value.as_str(), "pkg fun");
        assert_eq!(tab_diagnostics.len(), 1);

        let (unicode_tags, unicode_diagnostics, unicode_source) = parsed(
            r"#' @importFrom pkg fun
#' λcontinuation
",
            UnknownTagPolicy::Warn,
        );
        assert!(matches!(unicode_tags[0], ParsedTag::Namespace(_)));
        let diagnostic = unicode_diagnostics.iter().next().expect("diagnostic");
        assert_eq!(
            unicode_source.text_range(diagnostic.primary.span.range),
            Some("λ")
        );
    }
}
