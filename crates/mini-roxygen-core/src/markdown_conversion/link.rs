//! Lower Markdown links into the canonical Rd link and URL shapes.

use rd_ast::{RdNode, RdTag};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Label, Severity};
use crate::markdown::ROXYGEN_LINK_MARKER;

use super::frame::NodeWithOrigin;
use super::{Converter, LinkResolution};

fn text_leaves(value: &str, spans: Vec<crate::source::Span>) -> Vec<NodeWithOrigin> {
    super::leaf::physical_line_chunks(value)
        .map(|line| NodeWithOrigin {
            node: RdNode::Text(line.to_owned()),
            children: Vec::new(),
            spans: spans.clone(),
        })
        .collect()
}

fn verb_leaves(value: &str, spans: Vec<crate::source::Span>) -> Vec<NodeWithOrigin> {
    super::leaf::physical_line_chunks(value)
        .map(|line| NodeWithOrigin {
            node: RdNode::Verb(line.to_owned()),
            children: Vec::new(),
            spans: spans.clone(),
        })
        .collect()
}

pub(super) fn lower_link(
    converter: &mut Converter<'_>,
    destination: &str,
    start: usize,
    end: usize,
    children: Vec<NodeWithOrigin>,
) -> Vec<NodeWithOrigin> {
    let spans = converter.spans(start, end);
    let display_text = rendered_text(&children);

    let Some(destination) = destination.strip_prefix(ROXYGEN_LINK_MARKER) else {
        return vec![lower_url(destination, display_text, children, spans)];
    };

    let destination = percent_decode(destination);
    let code_span = children.len() == 1 && is_code_span(&children[0].node);
    let topic_input = if code_span {
        strip_code_delimiters(&destination)
    } else {
        destination.as_str()
    };
    let generated = children_are_text(&children) && display_text == topic_input;
    let parsed = parse_topic(topic_input);
    let mut package = parsed.package.map(str::to_owned);
    let mut explicit_package = package.is_some();
    let mut resolve_unqualified = package.is_none();

    if let Some(explicit) = package.as_deref()
        && converter.context.current_package == Some(explicit)
    {
        package = None;
        explicit_package = false;
        resolve_unqualified = false;
    }

    if resolve_unqualified {
        match converter.context.links.resolve_unqualified(parsed.topic) {
            LinkResolution::Local | LinkResolution::Unchecked => {}
            LinkResolution::External { package: resolved } => package = Some(resolved),
            LinkResolution::Unresolved => {
                warn_link(
                    converter,
                    start,
                    end,
                    format!("could not resolve Markdown help topic {:?}", parsed.topic),
                );
            }
            LinkResolution::Ambiguous { packages } => {
                warn_link(
                    converter,
                    start,
                    end,
                    format!(
                        "Markdown help topic {:?} has ambiguous packages: {}",
                        parsed.topic,
                        packages.join(", ")
                    ),
                );
            }
        }
    }

    let generated_label = if generated {
        let mut label = parsed.function.to_owned();
        if parsed.s4_class {
            label = label.strip_suffix("-class").unwrap_or(&label).to_owned();
        }
        if explicit_package {
            label = format!("{}::{label}", package.as_deref().unwrap_or_default());
        }
        label
    } else {
        display_text.clone()
    };

    // A rebuilt display loses the label's own markup, so keep the original
    // nodes around: they are what recovery has to fall back on.
    let (display, label) = if generated || code_span {
        let leaves = text_leaves(
            &generated_label,
            children
                .first()
                .map(|child| child.spans.clone())
                .unwrap_or_else(|| spans.clone()),
        );
        (leaves, Some(children))
    } else {
        (children, None)
    };
    let code = code_span || (generated && parsed.function.ends_with("()"));
    let topic = parsed.topic.to_owned();
    let option = match package {
        Some(package) => Some(format!("{package}:{topic}")),
        None if generated_label == topic => None,
        None => Some(format!("={topic}")),
    };
    // An Rd link option ends at the first `]`, so a target carrying one cannot
    // be written at all — R rejects the result too. A carriage return has no
    // representable leaf. Keep the prose and refuse only the link.
    if let Some(value) = option.as_deref()
        && value
            .chars()
            .any(|character| character == ']' || (character.is_control() && character != '\n'))
    {
        converter.diagnostics.push(Diagnostic::new(
            Severity::Error,
            DiagnosticCode::UnsupportedMarkdownConstruct,
            format!("Markdown help topic {topic:?} cannot be written as an Rd link target"),
            Label::new(
                converter.anchor(start).unwrap_or_else(|| spans[0]),
                "an Rd link target cannot contain `]` or a control character",
            ),
        ));
        // The link is gone, but a code-span label must stay code: hand back the
        // nodes the label came in with. A generated label cannot reach here —
        // it equals the target, and no Markdown label renders `]` or a control
        // character — so the rebuilt display is only ever plain text.
        return match label {
            Some(label) if code_span => label,
            _ => display,
        };
    }
    let option_nodes = option.as_deref().map(|value| {
        super::leaf::physical_line_chunks(value)
            .map(|line| RdNode::Text(line.to_owned()))
            .collect()
    });
    let link = NodeWithOrigin {
        node: RdNode::tagged(
            RdTag::Link,
            option_nodes,
            display.iter().map(|child| child.node.clone()).collect(),
        ),
        children: display,
        spans: spans.clone(),
    };
    if code {
        vec![NodeWithOrigin {
            node: RdNode::tagged(RdTag::Code, None, vec![link.node.clone()]),
            children: vec![link],
            spans,
        }]
    } else {
        vec![link]
    }
}

fn lower_url(
    destination: &str,
    display_text: String,
    children: Vec<NodeWithOrigin>,
    spans: Vec<crate::source::Span>,
) -> NodeWithOrigin {
    if destination.is_empty() || destination == display_text {
        let display_spans = children
            .first()
            .map(|child| child.spans.clone())
            .unwrap_or_else(|| spans.clone());
        let display = verb_leaves(&display_text, display_spans);
        NodeWithOrigin {
            node: RdNode::tagged(
                RdTag::Url,
                None,
                display.iter().map(|child| child.node.clone()).collect(),
            ),
            children: display,
            spans,
        }
    } else {
        let url = verb_leaves(destination, spans.clone());
        let url_nodes = url
            .iter()
            .map(|child| child.node.clone())
            .collect::<Vec<_>>();
        let display_group = NodeWithOrigin {
            node: RdNode::group(children.iter().map(|child| child.node.clone()).collect()),
            children,
            spans: spans.clone(),
        };
        NodeWithOrigin {
            node: RdNode::tagged(
                RdTag::Href,
                None,
                vec![RdNode::group(url_nodes), display_group.node.clone()],
            ),
            children: vec![
                NodeWithOrigin {
                    node: RdNode::group(url.iter().map(|child| child.node.clone()).collect()),
                    children: url,
                    spans: spans.clone(),
                },
                display_group,
            ],
            spans,
        }
    }
}

struct ParsedTopic<'a> {
    package: Option<&'a str>,
    function: &'a str,
    topic: &'a str,
    s4_class: bool,
}

fn parse_topic(destination: &str) -> ParsedTopic<'_> {
    let (package, function) = destination
        .split_once("::")
        .map_or((None, destination), |(package, function)| {
            (Some(package), function)
        });
    let topic = function.strip_suffix("()").unwrap_or(function);
    ParsedTopic {
        package,
        function,
        topic,
        s4_class: function.ends_with("-class"),
    }
}

fn is_code_span(node: &RdNode) -> bool {
    node.as_tagged()
        .is_some_and(|tagged| matches!(tagged.tag(), RdTag::Code | RdTag::Verb))
}

fn children_are_text(children: &[NodeWithOrigin]) -> bool {
    children
        .iter()
        .all(|child| matches!(child.node, RdNode::Text(_)))
}

fn rendered_text(children: &[NodeWithOrigin]) -> String {
    children.iter().map(rendered_node).collect()
}

fn rendered_node(node: &NodeWithOrigin) -> String {
    match &node.node {
        RdNode::Text(text) | RdNode::RCode(text) | RdNode::Verb(text) => text.clone(),
        RdNode::Tagged(_) | RdNode::Group(_) | RdNode::Comment(_) | RdNode::Raw(_) => {
            node.children.iter().map(rendered_node).collect()
        }
        _ => String::new(),
    }
}

fn strip_code_delimiters(destination: &str) -> &str {
    destination
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .unwrap_or(destination)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn warn_link(converter: &mut Converter<'_>, start: usize, end: usize, message: String) {
    let Some(primary) = converter.spans(start, end).first().copied() else {
        return;
    };
    converter.diagnostics.push(Diagnostic::new(
        Severity::Warning,
        DiagnosticCode::AmbiguousExternalAlias,
        message,
        Label::new(primary, "Markdown help link"),
    ));
}

#[cfg(test)]
mod tests {
    use super::super::frame::NodeWithOrigin;
    use super::super::test_support::{assert_serialized_body, serialize, value};
    use super::super::{
        HelpLinkResolver, LinkResolution, MarkdownContext, MarkdownConversion, convert_markdown,
    };
    use crate::diagnostic::Severity;
    use rd_ast::{RdNode, RdTag};

    #[derive(Clone)]
    enum Resolution {
        Local,
        Unchecked,
        External(&'static str),
        Unresolved,
        Ambiguous,
    }

    struct FakeResolver {
        resolution: Resolution,
    }

    impl HelpLinkResolver for FakeResolver {
        fn resolve_unqualified(&self, _topic: &str) -> LinkResolution {
            match &self.resolution {
                Resolution::Local => LinkResolution::Local,
                Resolution::Unchecked => LinkResolution::Unchecked,
                Resolution::External(package) => LinkResolution::External {
                    package: (*package).to_owned(),
                },
                Resolution::Unresolved => LinkResolution::Unresolved,
                Resolution::Ambiguous => LinkResolution::Ambiguous {
                    packages: vec!["first".to_owned(), "second".to_owned()],
                },
            }
        }
    }

    fn convert(
        markdown: &str,
        resolver: &FakeResolver,
        current_package: Option<&str>,
    ) -> MarkdownConversion {
        let context = MarkdownContext {
            current_package,
            links: resolver,
            inline_r_session: None,
        };
        convert_markdown(&value(markdown), &context)
    }

    fn local() -> FakeResolver {
        FakeResolver {
            resolution: Resolution::Local,
        }
    }

    fn link(option: Option<&str>, display: &str) -> RdNode {
        RdNode::tagged(
            RdTag::Link,
            option.map(|value| vec![RdNode::Text(value.to_owned())]),
            vec![RdNode::Text(display.to_owned())],
        )
    }

    fn code_link(option: Option<&str>, display: &str) -> RdNode {
        RdNode::tagged(RdTag::Code, None, vec![link(option, display)])
    }

    #[test]
    fn local_reference_forms_match_roxygen_lowering() {
        let resolver = local();
        for (markdown, expected) in [
            ("[obj]", link(None, "obj")),
            ("[`obj`]", code_link(None, "obj")),
            ("[fun()]", code_link(Some("=fun"), "fun()")),
            ("[text][obj]", link(Some("=obj"), "text")),
            ("[text][fun()]", link(Some("=fun"), "text")),
            ("[`text`][obj]", code_link(Some("=obj"), "text")),
            ("[s4-class]", link(Some("=s4-class"), "s4")),
        ] {
            let conversion = convert(markdown, &resolver, None);
            assert_eq!(
                conversion.fragment.nodes,
                vec![expected],
                "lowering {markdown}"
            );
            assert!(conversion.diagnostics.is_empty(), "diagnosing {markdown}");
        }
    }

    #[test]
    fn roxygen_quirks_are_replicated_on_purpose() {
        // These four forms look wrong, and an automated reviewer flagged two of
        // them as bugs. They are what roxygen2 8.1.0 actually emits, verified by
        // calling its internal markdown() on each input under R 4.6.1. Parity is
        // the goal, so matching it is correct and "fixing" any of these would
        // introduce a divergence. Do not change one without re-running roxygen2.
        //
        // roxygen2 decides whether a label was written or generated by comparing
        // the label text against the destination, not by the link's shortcut or
        // reference form. So a reference link whose label repeats its target
        // counts as generated, and picks up the code wrapping and the -class
        // trimming that a generated label gets.
        let resolver = local();
        for (markdown, expected) in [
            ("[fun()][fun()]", code_link(Some("=fun"), "fun()")),
            ("[s4-class][s4-class]", link(Some("=s4-class"), "s4")),
            ("[obj][obj]", link(None, "obj")),
            // roxygen2 strips exactly one backtick from each end of a code-span
            // target and applies none of CommonMark's code-span normalization,
            // so the surplus delimiters survive into the link target and the
            // link does not resolve. Reported upstream; we follow until it is
            // fixed, at which point this expectation changes with it.
            ("[``foo``]", code_link(Some("=`foo`"), "foo")),
        ] {
            let conversion = convert(markdown, &resolver, None);
            assert_eq!(
                conversion.fragment.nodes,
                vec![expected],
                "lowering {markdown}"
            );
        }
    }

    #[test]
    fn link_labels_match_roxygen_on_partial_code_and_markup() {
        // Cross-checked against roxygen2's own markdown-link tests. A label is
        // wrapped in \code only when it is entirely one code span, not when it
        // merely contains one.
        let resolver = local();
        for (markdown, expected) in [
            ("[`foo` bar][x]", r"\link[=x]{\code{foo} bar}"),
            ("[__baz__][x]", r"\link[=x]{\strong{baz}}"),
            ("[x][%%]", r"\link[=\%\%]{x}"),
            // Divergence: roxygen2 leaves balanced braces unescaped in a text
            // label, giving `foo({ bar })`. We escape them, because this layer
            // never escapes and the writer escapes braces in every text
            // context. Same rendered text; the Rd AST differs, since roxygen2's
            // form parses as a nested group and ours as literal characters.
            ("[foo({ bar })][x]", r"\link[=x]{foo(\{ bar \})}"),
        ] {
            let conversion = convert(markdown, &resolver, None);
            assert_serialized_body(conversion.fragment.nodes, expected);
        }
    }

    #[test]
    fn external_reference_forms_keep_package_qualified_generated_labels() {
        let resolver = local();
        for (markdown, expected) in [
            ("[pkg::obj]", link(Some("pkg:obj"), "pkg::obj")),
            ("[pkg::fun()]", code_link(Some("pkg:fun"), "pkg::fun()")),
            ("[text][pkg::obj]", link(Some("pkg:obj"), "text")),
            ("[text][pkg::fun()]", link(Some("pkg:fun"), "text")),
            ("[pkg::s4-class]", link(Some("pkg:s4-class"), "pkg::s4")),
        ] {
            let conversion = convert(markdown, &resolver, None);
            assert_eq!(
                conversion.fragment.nodes,
                vec![expected],
                "lowering {markdown}"
            );
            assert!(conversion.diagnostics.is_empty(), "diagnosing {markdown}");
        }
    }

    #[test]
    fn current_package_and_unqualified_external_resolution_are_normalized() {
        let resolver = FakeResolver {
            resolution: Resolution::External("otherpkg"),
        };
        assert_eq!(
            convert("[mypkg::fun()]", &resolver, Some("mypkg"))
                .fragment
                .nodes,
            vec![code_link(Some("=fun"), "fun()")]
        );
        assert_eq!(
            convert("[text][mypkg::obj]", &resolver, Some("mypkg"))
                .fragment
                .nodes,
            vec![link(Some("=obj"), "text")]
        );

        assert_eq!(
            convert("[obj]", &resolver, None).fragment.nodes,
            vec![link(Some("otherpkg:obj"), "obj")]
        );
    }

    #[test]
    fn unresolved_and_ambiguous_topics_remain_help_links_and_warn() {
        for resolution in [Resolution::Unresolved, Resolution::Ambiguous] {
            let resolver = FakeResolver { resolution };
            let conversion = convert("[obj]", &resolver, None);
            assert_eq!(conversion.fragment.nodes, vec![link(None, "obj")]);
            assert_eq!(conversion.diagnostics.len(), 1);
            assert_eq!(
                conversion
                    .diagnostics
                    .iter()
                    .next()
                    .expect("link warning")
                    .severity,
                Severity::Warning
            );
        }
    }

    #[test]
    fn unchecked_topics_keep_help_links_without_warning() {
        let conversion = convert(
            "[obj]",
            &FakeResolver {
                resolution: Resolution::Unchecked,
            },
            None,
        );
        assert_eq!(conversion.fragment.nodes, vec![link(None, "obj")]);
        assert!(conversion.diagnostics.is_empty());
    }

    #[test]
    fn ordinary_urls_use_url_or_two_group_href_shapes() {
        let resolver = local();
        let href = convert("[text](https://example.test)", &resolver, None);
        assert_eq!(
            href.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Href,
                None,
                vec![
                    RdNode::group(vec![RdNode::Verb("https://example.test".into())]),
                    RdNode::group(vec![RdNode::Text("text".into())]),
                ],
            )]
        );
        let angle = convert("<https://example.test>", &resolver, None);
        assert_eq!(
            angle.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Url,
                None,
                vec![RdNode::Verb("https://example.test".into())],
            )]
        );
        let equal = convert(
            "[https://example.test](https://example.test)",
            &resolver,
            None,
        );
        assert_eq!(
            equal.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Url,
                None,
                vec![RdNode::Verb("https://example.test".into())],
            )]
        );
        let empty = convert("[https://example.test]()", &resolver, None);
        assert_eq!(empty.fragment.nodes, equal.fragment.nodes);
        assert_serialized_body(href.fragment.nodes, r"\href{https://example.test}{text}");
        assert_serialized_body(angle.fragment.nodes, r"\url{https://example.test}");
    }

    #[test]
    fn nested_markdown_markup_remains_nested_around_links() {
        let resolver = local();
        let link_inside_emphasis = convert("*[obj]*", &resolver, None);
        assert_eq!(
            link_inside_emphasis.fragment.nodes,
            vec![RdNode::tagged(RdTag::Emph, None, vec![link(None, "obj")],)]
        );

        let emphasis_inside_link = convert("[**text**](https://example.test)", &resolver, None);
        assert_eq!(
            emphasis_inside_link.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Href,
                None,
                vec![
                    RdNode::group(vec![RdNode::Verb("https://example.test".into())]),
                    RdNode::group(vec![RdNode::tagged(
                        RdTag::Strong,
                        None,
                        vec![RdNode::Text("text".into())],
                    )]),
                ],
            )]
        );
    }

    #[test]
    fn url_verbatim_escaping_and_section_splitting_are_preserved() {
        let resolver = local();
        let conversion = convert("[text](https://example.test/%{raw})", &resolver, None);
        assert_serialized_body(
            conversion.fragment.nodes,
            r"\href{https://example.test/\%\{raw\}}{text}",
        );

        assert_eq!(
            crate::tags::test_support::split_parts("[base::split()] helper: body"),
            ("[base::split()] helper", " body")
        );
        let conversion = convert("[base::split()] helper: body", &resolver, None);
        assert!(matches!(
            conversion.fragment.nodes.first(),
            Some(RdNode::Tagged(_))
        ));
    }

    #[test]
    fn serialized_help_links_are_accepted_by_writer_and_r() {
        let resolver = local();
        assert_serialized_body(
            convert("[obj]", &resolver, None).fragment.nodes,
            r"\link{obj}",
        );
        assert_serialized_body(
            convert("[pkg::obj]", &resolver, None).fragment.nodes,
            r"\link[pkg:obj]{pkg::obj}",
        );
    }

    #[test]
    fn multiline_shortcut_help_links_split_display_and_option() {
        let resolver = local();
        let conversion = convert("[wrap\nped]", &resolver, None);
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Link,
                None,
                vec![RdNode::Text("wrap\n".into()), RdNode::Text("ped".into())],
            )]
        );
        assert_serialized_body(conversion.fragment.nodes, "\\link{wrap\nped}");

        let conversion = convert("[pkg::wrap\nped]", &resolver, None);
        assert_eq!(
            conversion.fragment.nodes,
            vec![RdNode::tagged(
                RdTag::Link,
                Some(vec![
                    RdNode::Text("pkg:wrap\n".into()),
                    RdNode::Text("ped".into()),
                ]),
                vec![
                    RdNode::Text("pkg::wrap\n".into()),
                    RdNode::Text("ped".into()),
                ],
            )]
        );
        assert_eq!(
            serialize(conversion.fragment.nodes),
            "\\link[pkg:wrap\nped]{pkg::wrap\nped}"
        );
    }

    #[test]
    fn multiline_urls_split_url_and_href_verbatim_children() {
        let url = super::lower_url(
            "https://example.test/a\nb",
            "https://example.test/a\nb".to_owned(),
            Vec::new(),
            Vec::new(),
        );
        assert_serialized_body(vec![url.node.clone()], "\\url{https://example.test/a\nb}");

        let href = super::lower_url(
            "https://example.test/a\nb",
            "label".to_owned(),
            vec![NodeWithOrigin {
                node: RdNode::Text("label".into()),
                children: Vec::new(),
                spans: Vec::new(),
            }],
            Vec::new(),
        );
        assert_serialized_body(vec![href.node], "\\href{https://example.test/a\nb}{label}");
    }
}
