//! Semantic tag data types and their source-aware values.

use super::registry::UnsupportedTagKind;
#[cfg(test)]
use super::text::NormalizeHead;
use super::text::SourcedText;
use super::words::parse_words;
use crate::source::{Span, Spanned, TextRange};

/// Identifies whether a semantic value came from an explicit tag or an intro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagOrigin {
    /// The value was written after a named tag.
    Explicit {
        /// The source-backed tag name.
        name: Spanned<String>,
        /// The raw source span after the tag name.
        value_span: Span,
        /// The complete source span of the tag section.
        full_span: Span,
    },
    /// The value was synthesized from an untagged intro.
    Implicit {
        /// The source span of the intro section.
        intro_span: Span,
    },
}

/// A semantic value together with the source origin that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagValue<T> {
    /// The parsed value.
    pub value: T,
    /// The value's source origin.
    pub origin: TagOrigin,
}

/// Describes whether a field contributes a value to the generated topic.
///
/// The suppression state is typed at tag parsing time because roxygen2 treats
/// an exact plain `NULL` value as control data rather than prose. Keeping that
/// distinction here prevents later merge layers from comparing rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue<T> {
    /// Emit the parsed field value.
    Emit(T),
    /// Suppress this field's contribution.
    Suppress,
}

/// A source-aware field value that may explicitly suppress output.
///
/// The alias makes the sentinel-bearing nature visible in every prose tag's
/// public type while retaining the common tag-origin representation.
pub type FieldTag<T> = TagValue<FieldValue<T>>;

/// Normalized text that a later layer will interpret as Markdown.
///
/// This is not a Markdown AST. It can also represent empty text; whether a
/// particular tag requires content is a validation rule for that tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownText(SourcedText);

impl MarkdownText {
    /// Creates Markdown text from normalized, provenance-carrying text.
    #[must_use]
    pub const fn new(value: SourcedText) -> Self {
        Self(value)
    }

    /// Returns the normalized text awaiting Markdown interpretation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns whether the normalized text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the source span of the whole character at a normalized byte offset.
    #[must_use]
    pub fn source_span_at(&self, offset: u32) -> Option<Span> {
        self.0.source_span_at(offset)
    }

    /// Returns a zero-width source span at a normalized byte offset.
    #[must_use]
    pub fn source_anchor_at(&self, offset: u32) -> Option<Span> {
        self.0.source_anchor_at(offset)
    }

    /// Returns source spans represented by a normalized byte range.
    #[must_use]
    pub fn source_spans(&self, range: TextRange) -> Vec<Span> {
        self.0.source_spans(range)
    }

    /// Borrows the provenance-carrying text.
    #[must_use]
    pub const fn sourced(&self) -> &SourcedText {
        &self.0
    }
}

/// Normalized text that later layers must keep as plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainText(SourcedText);

impl PlainText {
    /// Creates plain text from normalized, provenance-carrying text.
    #[must_use]
    pub const fn new(value: SourcedText) -> Self {
        Self(value)
    }

    /// Returns the normalized text without Markdown interpretation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrows the provenance-carrying text.
    #[must_use]
    pub const fn sourced(&self) -> &SourcedText {
        &self.0
    }

    /// Returns whitespace-separated words with their source spans.
    #[must_use]
    pub fn words(&self) -> Vec<Spanned<String>> {
        parse_words(self.sourced(), |word| word.to_owned())
    }
}

/// Normalized R source that must not be evaluated or reformatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RCodeText(SourcedText);

impl RCodeText {
    /// Creates R source from normalized, provenance-carrying text.
    #[must_use]
    pub const fn new(value: SourcedText) -> Self {
        Self(value)
    }

    /// Returns the normalized R source verbatim.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns whether the normalized source is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the source span of the whole character at a normalized byte offset.
    #[must_use]
    pub fn source_span_at(&self, offset: u32) -> Option<Span> {
        self.0.source_span_at(offset)
    }

    /// Returns a zero-width source span at a normalized byte offset.
    #[must_use]
    pub fn source_anchor_at(&self, offset: u32) -> Option<Span> {
        self.0.source_anchor_at(offset)
    }

    /// Returns source spans represented by a normalized byte range.
    #[must_use]
    pub fn source_spans(&self, range: TextRange) -> Vec<Span> {
        self.0.source_spans(range)
    }

    /// Borrows the provenance-carrying source.
    #[must_use]
    pub const fn sourced(&self) -> &SourcedText {
        &self.0
    }
}

/// A statically validated conditional examples block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamplesIf {
    /// The condition expression from the first line of the conditional tag.
    pub condition: RCodeText,
    /// Example source following the condition line.
    pub body: RCodeText,
}

/// The typed content variants accepted by the examples section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExamplesContent {
    /// Ordinary example source.
    Ordinary(RCodeText),
    /// Example source guarded by a static condition.
    Conditional(ExamplesIf),
}

/// A documented parameter name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamName(pub String);

/// A documentation topic name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocName(pub String);

/// A documentation keyword.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Keyword(pub String);

/// A source topic used by inheritance tags.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopicRef(pub String);

/// The explicitly requested behavior of an `@usage` tag.
///
/// `NULL` must remain distinguishable from absent `@usage`, because it
/// suppresses generated usage for this block instead of merely providing no
/// explicit text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageDirective {
    /// Emit the supplied R source verbatim.
    Explicit(RCodeText),
    /// Suppress usage that would otherwise be generated.
    SuppressGenerated,
}

/// The aliases explicitly supplied by one `@aliases` tag and its default policy.
///
/// The policy is kept alongside the explicit aliases because `NULL` changes
/// only the block's default aliases; it is not itself an alias word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasDirective {
    /// Explicit aliases other than the control word `NULL`.
    pub explicit: Vec<Spanned<DocName>>,
    /// Whether this block contributes its default aliases.
    pub defaults: DefaultAliasPolicy,
}

/// Controls whether an aliases tag contributes the object's default aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAliasPolicy {
    /// Include the primary and implicit object aliases.
    Include,
    /// Suppress the primary and implicit object aliases.
    Suppress,
}

/// The target requested by an inheritance tag.
///
/// A suppression is retained as a target because a later merge layer may need
/// it to cancel an inheritance request made by another documentation block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritTarget {
    /// Inherit from the named topic.
    Topic(Spanned<TopicRef>),
    /// Suppress inheritance from this tag at the given source span.
    Suppress(Span),
}

/// One component that `@inherit` can copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InheritField {
    /// Parameter documentation.
    Params,
    /// Return documentation.
    Return,
    /// Title documentation.
    Title,
    /// Description documentation.
    Description,
    /// Details documentation.
    Details,
    /// See-also documentation.
    SeeAlso,
    /// Named sections.
    Sections,
    /// References documentation.
    References,
    /// Examples source.
    Examples,
    /// Author documentation.
    Author,
    /// Source documentation.
    Source,
    /// Note documentation.
    Note,
    /// Format documentation.
    Format,
}

/// The components selected by an `@inherit` tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritFields {
    /// Inherit every supported component.
    All {
        /// A source span identifying the all-components selection.
        anchor: Span,
    },
    /// Inherit only the listed components.
    Selected(Vec<Spanned<InheritField>>),
}

/// One selector in the deliberately narrow `@inheritParams` selection grammar.
///
/// mini-roxygen supports `@inheritParams` filters consisting of parameter names
/// and directly negated parameter names, such as `x z` or `-y`. Unlike
/// roxygen2, it does not evaluate numeric selectors, ranges, parentheses,
/// subtraction, or other R expressions in selection tails. An unsupported
/// selector is an error and its inheritance request is skipped; it is never
/// reinterpreted and never treated as an unfiltered request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgSelector {
    /// Select one named argument.
    Name(Spanned<ParamName>),
    /// Remove a selector from the result.
    Exclude(Spanned<ParamName>),
}

/// The parsed shape of an `@inheritParams` selection tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgSelection {
    /// Selectors retained for a later, deliberately separate evaluator.
    pub selectors: Vec<ArgSelector>,
}

/// A tag not recognized by this semantic slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTag {
    /// The source-backed tag name.
    pub name: Spanned<String>,
    /// The normalized value, retaining source provenance.
    pub value: SourcedText,
    /// The raw source span after the tag name.
    pub value_span: Span,
    /// The complete source span of the tag section.
    pub full_span: Span,
}

/// A recognized NAMESPACE tag whose value remains opaque for a later lowering layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceTag {
    /// An export directive.
    Export(TagValue<PlainText>),
    /// An S3 method export directive.
    ExportS3Method(TagValue<PlainText>),
    /// A package import directive.
    Import(TagValue<PlainText>),
    /// A package-qualified import directive.
    ImportFrom(TagValue<PlainText>),
    /// Raw NAMESPACE source.
    RawNamespace(TagValue<PlainText>),
    /// A dynamic-library import directive.
    UseDynLib(TagValue<PlainText>),
    /// An export-pattern directive.
    ExportPattern(TagValue<PlainText>),
    /// An S4 class export directive.
    ExportClass(TagValue<PlainText>),
    /// An S4 method export directive.
    ExportMethod(TagValue<PlainText>),
    /// An S4 class import directive.
    ImportClassesFrom(TagValue<PlainText>),
    /// An S4 method import directive.
    ImportMethodsFrom(TagValue<PlainText>),
}

/// A recognized tag that requires R evaluation and therefore cannot be lowered here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedTag {
    /// The unsupported operation requested by the tag.
    pub kind: UnsupportedTagKind,
    /// The normalized value and its source origin.
    pub value: TagValue<PlainText>,
}

/// The closed set of semantic tags currently implemented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTag {
    /// An explicit title whose content is normalized Markdown text.
    Title(FieldTag<MarkdownText>),
    /// A description whose content is normalized Markdown text.
    Description(FieldTag<MarkdownText>),
    /// Details whose content is normalized Markdown text.
    Details(FieldTag<MarkdownText>),
    /// Return information whose content is normalized Markdown text.
    Return(FieldTag<MarkdownText>),
    /// Cross-references whose content is normalized Markdown text.
    SeeAlso(FieldTag<MarkdownText>),
    /// Bibliographic references whose content is normalized Markdown text.
    References(FieldTag<MarkdownText>),
    /// A note whose content is normalized Markdown text.
    Note(FieldTag<MarkdownText>),
    /// Format information whose content is normalized Markdown text.
    Format(FieldTag<MarkdownText>),
    /// Source information whose content is normalized Markdown text.
    Source(FieldTag<MarkdownText>),
    /// Author information whose content is normalized Markdown text.
    Author(FieldTag<MarkdownText>),
    /// One or more documented parameters with a shared Markdown description.
    Param {
        /// Parameter names, each retaining its own source span.
        names: Vec<Spanned<ParamName>>,
        /// The shared parameter description.
        description: MarkdownText,
        /// The source origin of the complete tag value.
        origin: TagOrigin,
    },
    /// A plain documentation topic name.
    Name(TagValue<PlainText>),
    /// A plain Rd file name.
    RdName(TagValue<PlainText>),
    /// Documentation aliases split into source-backed words.
    Aliases(TagValue<AliasDirective>),
    /// Documentation keywords split into source-backed words.
    Keywords(FieldTag<Vec<Spanned<Keyword>>>),
    /// Suppresses Rd output for the documented object.
    NoRd(TagOrigin),
    /// Typed content used in the examples section.
    Examples(TagValue<ExamplesContent>),
    /// R source used as usage, without evaluation or reformatting.
    Usage(TagValue<UsageDirective>),
    /// An S3 method declaration for Rd and usage generation.
    Method {
        /// The S3 generic name.
        generic: Spanned<String>,
        /// The S3 class name.
        class: Spanned<String>,
        /// The source origin of the complete tag value.
        origin: TagOrigin,
    },
    /// An ordering value for blocks merged into one topic.
    Order {
        /// The integer ordering value.
        value: i64,
        /// The source origin of the complete tag value.
        origin: TagOrigin,
    },
    /// Requests inheritance of selected documentation components.
    Inherit {
        /// The source topic or an explicit suppression.
        target: InheritTarget,
        /// Components to inherit, including an explicit all-components state.
        fields: InheritFields,
        /// The source origin of the tag value.
        origin: TagOrigin,
    },
    /// Requests inheritance of one named section from a source topic.
    InheritSection {
        /// The source topic.
        target: InheritTarget,
        /// The section title, retaining its source span.
        title: Spanned<MarkdownText>,
        /// The source origin of the tag value.
        origin: TagOrigin,
    },
    /// Requests inheritance of all parameters from a source topic.
    InheritParams {
        /// The source topic or an explicit suppression.
        target: InheritTarget,
        /// A deferred selection expression, absent for all parameters.
        selection: Option<ArgSelection>,
        /// The source origin of the tag value.
        origin: TagOrigin,
    },
    /// A section split into source-level Markdown title and body.
    Section {
        /// The section title before the first eligible colon.
        title: MarkdownText,
        /// The section body after the first eligible colon.
        body: MarkdownText,
        /// The source origin of the complete section value.
        origin: TagOrigin,
    },
    /// A recognized NAMESPACE directive awaiting typed lowering.
    Namespace(NamespaceTag),
    /// A recognized R-evaluated tag retained for diagnostics and inspection.
    Unsupported(UnsupportedTag),
    /// A tag awaiting a later registry slice.
    Unknown(UnknownTag),
}

/// Controls diagnostics for tags outside the recognized semantic set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum UnknownTagPolicy {
    /// Retain unknown tags without emitting diagnostics.
    Ignore,
    /// Retain unknown tags and emit a warning.
    #[default]
    Warn,
    /// Retain unknown tags and emit an error.
    Error,
}

/// Options controlling semantic tag parsing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TagParseOptions {
    /// Diagnostic behavior for tags not recognized in this slice.
    pub unknown_tags: UnknownTagPolicy,
}

impl TagParseOptions {
    /// Sets the diagnostic policy for tags not recognized in this slice.
    #[must_use]
    pub const fn with_unknown_tags(mut self, policy: UnknownTagPolicy) -> Self {
        self.unknown_tags = policy;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{MarkdownText, UsageDirective};
    use crate::diagnostic::DiagnosticCode;
    use crate::source::{FileId, SourceFile};
    use crate::tags::test_support::parsed;
    use crate::tags::{FieldValue, NamespaceTag, ParsedTag, TagOrigin, UnknownTagPolicy};

    #[test]
    fn markdown_text_can_be_empty() {
        let value = MarkdownText::new(super::SourcedText::from_lines(
            &SourceFile::new(PathBuf::from("test.R"), String::new()),
            &[],
            super::NormalizeHead::TagValue,
        ));
        assert!(value.is_empty());
        assert_eq!(value.as_str(), "");
    }

    #[test]
    fn title_is_normalized_trimmed_and_source_backed() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @title   Hello @@世界  
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(diagnostics.is_empty());
        let ParsedTag::Title(title) = &tags[0] else {
            panic!("expected title");
        };
        let FieldValue::Emit(title_value) = &title.value else {
            panic!("expected emitted title");
        };
        assert_eq!(title_value.as_str(), "Hello @世界");
        assert_eq!(
            title_value.source_span_at(6),
            Some(crate::source::Span::new(
                FileId::new(0),
                crate::source::TextRange::new(18, 20),
            ))
        );
        assert!(matches!(title.origin, TagOrigin::Explicit { .. }));
    }

    #[test]
    fn trimming_preserves_content_provenance() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @title   Hello   
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(diagnostics.is_empty());
        let ParsedTag::Title(title) = &tags[0] else {
            panic!("expected title");
        };
        let FieldValue::Emit(title_value) = &title.value else {
            panic!("expected emitted title");
        };
        assert_eq!(title_value.as_str(), "Hello");
        assert_eq!(
            title_value.source_spans(crate::source::TextRange::new(0, 5)),
            vec![crate::source::Span::new(
                FileId::new(0),
                crate::source::TextRange::new(12, 17),
            )]
        );
    }

    #[test]
    fn prose_tags_normalize_and_return_aliases_share_a_variant() {
        let (tags, diagnostics, _) = parsed(
            r"#' @description  Description @@text  
#' @details Details
#' @return Return
#' @returns Returns
#' @seealso See also
#' @references References
#' @note Note
#' @format Format
#' @source Source
#' @author Author
",
            UnknownTagPolicy::Warn,
        );

        assert!(diagnostics.is_empty());
        assert!(matches!(tags[0], ParsedTag::Description(_)));
        assert_eq!(prose_text(&tags[0]), "Description @text");
        assert!(matches!(tags[1], ParsedTag::Details(_)));
        assert!(matches!(tags[2], ParsedTag::Return(_)));
        assert!(matches!(tags[3], ParsedTag::Return(_)));
        assert!(matches!(tags[4], ParsedTag::SeeAlso(_)));
        assert!(matches!(tags[5], ParsedTag::References(_)));
        assert!(matches!(tags[6], ParsedTag::Note(_)));
        assert!(matches!(tags[7], ParsedTag::Format(_)));
        assert!(matches!(tags[8], ParsedTag::Source(_)));
        assert!(matches!(tags[9], ParsedTag::Author(_)));

        let ParsedTag::Return(return_tag) = &tags[2] else {
            panic!("expected return");
        };
        let TagOrigin::Explicit { name, .. } = &return_tag.origin else {
            panic!("expected explicit origin");
        };
        assert_eq!(name.value, "return");
        let ParsedTag::Return(returns_tag) = &tags[3] else {
            panic!("expected returns");
        };
        let TagOrigin::Explicit { name, .. } = &returns_tag.origin else {
            panic!("expected explicit origin");
        };
        assert_eq!(name.value, "returns");
    }

    #[test]
    fn simple_word_and_toggle_tags_are_structured() {
        let (tags, diagnostics, _) = parsed(
            r"#' @name topic
#' @rdname topic
#' @aliases first second
#' @keywords one two
#' @noRd
#' @noRd value
",
            UnknownTagPolicy::Warn,
        );
        assert!(matches!(tags[0], ParsedTag::Name(_)));
        assert!(matches!(tags[1], ParsedTag::RdName(_)));
        let ParsedTag::Aliases(directive) = &tags[2] else {
            panic!("expected aliases");
        };
        assert_eq!(
            directive
                .value
                .explicit
                .iter()
                .map(|value| &value.value.0)
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let ParsedTag::Keywords(directive) = &tags[3] else {
            panic!("expected keywords");
        };
        let FieldValue::Emit(values) = &directive.value else {
            panic!("expected emitted keywords");
        };
        assert_eq!(
            values
                .iter()
                .map(|value| &value.value.0)
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(matches!(tags[4], ParsedTag::NoRd(_)));
        assert_eq!(tags.len(), 5);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics.iter().next().expect("diagnostic").code,
            DiagnosticCode::TagParseError
        );
    }

    #[test]
    fn code_tags_preserve_indentation_and_only_one_example_newline() {
        let (tags, diagnostics, _) = parsed(
            r"#' @examples
#'   first()
#'
#'   second()
#' @usage  f(x)
#' continued
",
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Examples(examples) = &tags[0] else {
            panic!("expected examples");
        };
        let super::ExamplesContent::Ordinary(examples_value) = &examples.value else {
            panic!("expected ordinary examples");
        };
        assert_eq!(examples_value.as_str(), "  first()\n\n  second()");
        let ParsedTag::Usage(usage) = &tags[1] else {
            panic!("expected usage");
        };
        let UsageDirective::Explicit(usage_value) = &usage.value else {
            panic!("expected explicit usage");
        };
        assert_eq!(usage_value.as_str(), "f(x)\ncontinued");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn examples_if_keeps_condition_and_body_typed() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @examplesIf interactive()
#'   value <- 1
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Examples(examples) = &tags[0] else {
            panic!("expected examplesIf");
        };
        let super::ExamplesContent::Conditional(examples_value) = &examples.value else {
            panic!("expected conditional examples");
        };
        assert_eq!(examples_value.condition.as_str(), "interactive()");
        assert_eq!(examples_value.body.as_str(), "  value <- 1");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn examples_if_rejects_invalid_conditions_and_empty_bodies() {
        for (input, code) in [
            (
                r#"#' @examplesIf invalid(
#' value <- 1
"#,
                DiagnosticCode::InvalidExamplesIfCondition,
            ),
            (
                r#"#' @examplesIf interactive()
"#,
                DiagnosticCode::EmptyExamplesIfBody,
            ),
        ] {
            let (tags, diagnostics, _) = parsed(input, UnknownTagPolicy::Warn);
            assert!(tags.is_empty());
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics.iter().next().expect("diagnostic").code, code);
        }
    }

    #[test]
    fn namespace_tags_are_opaque_and_do_not_warn_as_unknown() {
        let (tags, diagnostics, source) = parsed(
            r"#' @export
#' @exportS3Method
#' @method generic class
#' @import foo,bar
#' @importFrom package fun,other
#' @rawNamespace import(foo)
#' @useDynLib package
#' @exportPattern ^foo
#' @exportClass Class
#' @exportMethod method
#' @importClassesFrom package Class
#' @importMethodsFrom package method
",
            UnknownTagPolicy::Warn,
        );
        assert!(diagnostics.is_empty());
        assert_eq!(tags.len(), 12);
        let ParsedTag::Method { generic, class, .. } = &tags[2] else {
            panic!("expected method");
        };
        assert_eq!(generic.value, "generic");
        assert_eq!(class.value, "class");
        assert_eq!(source.text_range(generic.span.range), Some("generic"));
        assert_eq!(source.text_range(class.span.range), Some("class"));
        let ParsedTag::Namespace(NamespaceTag::Import(import)) = &tags[3] else {
            panic!("expected import");
        };
        assert_eq!(import.value.as_str(), "foo,bar");
        assert!(
            tags[0..2]
                .iter()
                .all(|tag| matches!(tag, ParsedTag::Namespace(_)))
        );
        assert!(
            tags[3..]
                .iter()
                .all(|tag| matches!(tag, ParsedTag::Namespace(_)))
        );
    }

    #[test]
    fn raw_namespace_preserves_outer_internal_and_trailing_bytes() {
        let (tags, diagnostics, _) = parsed(
            concat!(
                r#"#' @rawNamespace
#' first
#'   middle"#,
                "  \n#' \n",
            ),
            UnknownTagPolicy::Warn,
        );
        assert!(diagnostics.is_empty());
        let ParsedTag::Namespace(NamespaceTag::RawNamespace(raw)) = &tags[0] else {
            panic!("expected raw namespace tag");
        };
        assert_eq!(raw.value.as_str(), "\nfirst\n  middle  \n");
    }

    #[test]
    fn method_requires_exactly_two_words() {
        let (one, one_diagnostics, _) = parsed(
            r#"#' @method print
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(one.is_empty());
        assert_eq!(one_diagnostics.len(), 1);

        let (three, three_diagnostics, _) = parsed(
            r#"#' @method print foo extra
"#,
            UnknownTagPolicy::Warn,
        );
        assert!(three.is_empty());
        assert_eq!(three_diagnostics.len(), 1);
    }

    #[test]
    fn order_is_an_integer_and_requires_a_value() {
        let (tags, diagnostics, _) = parsed(
            r#"#' @order 2
#' @order two
#' @order
"#,
            UnknownTagPolicy::Warn,
        );
        let ParsedTag::Order { value, .. } = tags.first().expect("order tag") else {
            panic!("expected order");
        };
        assert_eq!(*value, 2);
        assert_eq!(tags.len(), 1);
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "@order requires a single integer")
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "@order requires a value")
        );
    }

    fn prose_text(tag: &ParsedTag) -> &str {
        match tag {
            ParsedTag::Description(value)
            | ParsedTag::Details(value)
            | ParsedTag::Return(value)
            | ParsedTag::SeeAlso(value)
            | ParsedTag::References(value)
            | ParsedTag::Note(value)
            | ParsedTag::Format(value)
            | ParsedTag::Source(value)
            | ParsedTag::Author(value) => {
                let FieldValue::Emit(value) = &value.value else {
                    panic!("expected emitted prose");
                };
                value.as_str()
            }
            _ => panic!("expected prose tag"),
        }
    }
}
