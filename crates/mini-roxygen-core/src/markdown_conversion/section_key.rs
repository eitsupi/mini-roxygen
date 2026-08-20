//! Structural keys for comparing inherited section titles.
//!
//! A construct is flattened to visible text only when its visible-character
//! projection is known from the source alone. Everything else is encoded
//! structurally. This is a comparison key, not rendered text: dynamic,
//! format-dependent, and otherwise opaque output must retain its syntax.
//!
//! The structural encoder intentionally does not retain the leaf-kind
//! difference between `Text`, `Verb`, and `RCode` inside a structural
//! construct. Consequently, a non-canonical `\Sexpr{Text("A")}` keys the
//! same as the canonical `\Sexpr{RCode("A")}`. Real parses produce `RCode`
//! there; preserving this distinction would require context-sensitive
//! tokenization.

use rd_ast::{RdNode, RdPath, RdTag};

/// A flat, delimiter-free key for an inherited section title.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct SectionTitleKey(Vec<SectionKeyToken>);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum SectionKeyToken {
    /// Visible text. Adjacent runs are coalesced.
    Text(String),
    /// Opens a construct whose visible output is not known statically.
    Open(SectionKeyConstruct),
    /// The bracket option of the enclosing construct follows.
    Option,
    /// The nth positional argument of the enclosing construct follows.
    Argument(usize),
    Close,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum SectionKeyConstruct {
    /// A tag in `RdTag`'s closed vocabulary, by `as_rd_tag()`.
    KnownTag(String),
    /// `RdTag::Unknown`, by its stored string.
    UnknownTag(String),
    /// An `RdNode::Raw`, retaining its tag and opaque payload together.
    Raw { tag: Option<String>, opaque: String },
}

impl SectionTitleKey {
    pub(crate) fn from_rd(nodes: &[RdNode]) -> Self {
        let mut builder = KeyBuilder::default();
        builder.nodes(nodes, &RdPath::new(Vec::new()));
        Self(builder.tokens)
    }

    pub(crate) fn from_text(value: &str) -> Self {
        let mut builder = KeyBuilder::default();
        builder.text(value);
        Self(builder.tokens)
    }
}

#[derive(Default)]
struct KeyBuilder {
    tokens: Vec<SectionKeyToken>,
}

impl KeyBuilder {
    fn nodes(&mut self, nodes: &[RdNode], path: &RdPath) {
        for (index, node) in nodes.iter().enumerate() {
            self.node(node, &path.with_child(index));
        }
    }

    fn node(&mut self, node: &RdNode, path: &RdPath) {
        match node {
            // RCode is the visible content of transparent markup such as
            // \code, \special, \usage, and \examples. It is not literal
            // display inside \Sexpr, where it is code to be evaluated; that
            // case is safe because \Sexpr itself is retained structurally
            // from the outside, so this leaf never reaches a comparison
            // with a plain title.
            RdNode::Text(value) | RdNode::Verb(value) | RdNode::RCode(value) => self.text(value),
            // Comments are not part of rendered content. Dropping one does
            // not break text coalescing: the next text token can join the
            // preceding one.
            RdNode::Comment(_) => {}
            // A brace group is not visible in any position, including inside
            // a structural argument. Thus Group[Text("A")] keys exactly as
            // Argument(0) containing Text("A").
            RdNode::Group(group) => self.nodes(group.children(), path),
            RdNode::Raw(raw) => self.structural(
                SectionKeyConstruct::Raw {
                    tag: raw.tag().map(str::to_owned),
                    // RawRdNode's payload and attributes cannot be embedded
                    // in an Ord key (their types contain floats). This single
                    // debug-formatted tuple is deterministic and keeps the
                    // two parts associated. RawRdReal gives NA/NaN/infinity
                    // dedicated variants; Finite is documented as finite, so
                    // Debug spelling is exact for values honouring that rule.
                    opaque: format!("{:?}", (raw.payload(), raw.attributes())),
                },
                raw.option(),
                raw.children(),
                path,
            ),
            RdNode::Tagged(tagged) => self.tagged(node, tagged, path),
            // RdNode is non-exhaustive. A newly added leaf must not silently
            // become visible text and collide with an existing title.
            _ => self.structural_node(node, path),
        }
    }

    fn tagged(&mut self, node: &RdNode, tagged: &rd_ast::RdTagged, path: &RdPath) {
        // These view accessors are intentionally used only for the
        // transparent set. Every failed or unusable view falls through to
        // the same node's raw structural representation below.
        if let Some(view) = node.inline_span(path) {
            match view.kind() {
                // rd-ast groups these with uniform one-argument spans, but
                // rendering adds output-dependent quotation characters.
                rd_ast::RdInlineSpanKind::SQuote | rd_ast::RdInlineSpanKind::DQuote => {}
                rd_ast::RdInlineSpanKind::Emph
                | rd_ast::RdInlineSpanKind::Strong
                | rd_ast::RdInlineSpanKind::Bold
                | rd_ast::RdInlineSpanKind::Code
                | rd_ast::RdInlineSpanKind::Special
                | rd_ast::RdInlineSpanKind::Verb
                | rd_ast::RdInlineSpanKind::Url
                | rd_ast::RdInlineSpanKind::Email
                | rd_ast::RdInlineSpanKind::File
                | rd_ast::RdInlineSpanKind::Pkg
                | rd_ast::RdInlineSpanKind::Samp
                | rd_ast::RdInlineSpanKind::Kbd
                | rd_ast::RdInlineSpanKind::Var
                | rd_ast::RdInlineSpanKind::Env
                | rd_ast::RdInlineSpanKind::Command
                | rd_ast::RdInlineSpanKind::Option
                | rd_ast::RdInlineSpanKind::Acronym
                | rd_ast::RdInlineSpanKind::Abbr
                | rd_ast::RdInlineSpanKind::Cite
                | rd_ast::RdInlineSpanKind::Dfn => {
                    self.nodes(view.body(), path);
                    return;
                }
                // RdInlineSpanKind is non-exhaustive. New kinds must remain
                // structural until their visible projection is reviewed.
                _ => {}
            }
        }

        if let Some(view) = node.text_symbol(path) {
            match view.kind() {
                rd_ast::RdTextSymbolKind::R
                | rd_ast::RdTextSymbolKind::Dots
                | rd_ast::RdTextSymbolKind::LDots => {
                    self.text(view.fallback_text());
                    return;
                }
                _ => {}
            }
        }

        if tagged.tag() == &RdTag::I && tagged.option().is_none() {
            self.nodes(tagged.children(), path);
            return;
        }

        match tagged.tag() {
            RdTag::Link => {
                if let Ok(view) = tagged.inspect_link(path) {
                    self.nodes(view.display(), path);
                    return;
                }
            }
            RdTag::Href => {
                if let Ok(view) = tagged.inspect_href(path) {
                    self.nodes(view.display(), path);
                    return;
                }
            }
            _ => {}
        }

        if let Some(view) = node.s4_class_link(path) {
            self.nodes(view.class(), path);
            return;
        }

        self.structural_node(node, path);
    }

    fn structural_node(&mut self, node: &RdNode, path: &RdPath) {
        let Some(tagged) = node.as_tagged() else {
            // This arm is reachable only for a future RdNode variant. It has
            // no current representation to retain, so give it a distinct
            // non-text marker rather than flattening it.
            self.tokens
                .push(SectionKeyToken::Open(SectionKeyConstruct::Raw {
                    tag: None,
                    opaque: format!("{:?}", node),
                }));
            self.tokens.push(SectionKeyToken::Close);
            return;
        };
        let construct = match tagged.tag() {
            RdTag::Unknown(value) => SectionKeyConstruct::UnknownTag(value.clone()),
            tag => SectionKeyConstruct::KnownTag(tag.as_rd_tag().to_owned()),
        };
        self.structural(construct, tagged.option(), tagged.children(), path);
    }

    fn structural(
        &mut self,
        construct: SectionKeyConstruct,
        option: Option<&[RdNode]>,
        children: &[RdNode],
        path: &RdPath,
    ) {
        // In particular, Sexpr is compared syntactically. Donor provenance is
        // deliberately absent, so identical dynamic source compares equal as
        // it does in roxygen2; this key never evaluates the expression.
        self.tokens.push(SectionKeyToken::Open(construct));
        if let Some(option) = option {
            self.tokens.push(SectionKeyToken::Option);
            self.nodes(option, &path.with_option());
        }
        for (index, child) in children.iter().enumerate() {
            self.tokens.push(SectionKeyToken::Argument(index));
            self.node(child, &path.with_child(index));
        }
        self.tokens.push(SectionKeyToken::Close);
    }

    fn text(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        match self.tokens.last_mut() {
            Some(SectionKeyToken::Text(previous)) => previous.push_str(value),
            _ => self.tokens.push(SectionKeyToken::Text(value.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_presence_markers_preserve_empty_shapes() {
        let without_option = RdNode::tagged(RdTag::If, None, Vec::new());
        let with_empty_option = RdNode::tagged(RdTag::If, Some(Vec::new()), Vec::new());
        let with_empty_argument = RdNode::tagged(RdTag::If, None, vec![RdNode::group(Vec::new())]);
        assert_ne!(
            SectionTitleKey::from_rd(&[without_option]),
            SectionTitleKey::from_rd(&[with_empty_option]),
        );
        assert_ne!(
            SectionTitleKey::from_rd(&[RdNode::tagged(RdTag::If, None, Vec::new())]),
            SectionTitleKey::from_rd(&[with_empty_argument]),
        );
    }

    #[test]
    fn transparent_code_leaves_key_as_visible_text() {
        let code = RdNode::tagged(RdTag::Code, None, vec![RdNode::RCode("A".to_owned())]);
        let verb = RdNode::tagged(RdTag::Verb, None, vec![RdNode::Verb("A".to_owned())]);
        let plain = RdNode::Text("A".to_owned());

        assert_eq!(
            SectionTitleKey::from_rd(&[code]),
            SectionTitleKey::from_rd(std::slice::from_ref(&plain))
        );
        assert_eq!(
            SectionTitleKey::from_rd(&[verb]),
            SectionTitleKey::from_rd(std::slice::from_ref(&plain))
        );
    }

    #[test]
    fn visible_text_coalesces_across_leaf_kinds() {
        let mixed = [
            RdNode::Text("A".to_owned()),
            RdNode::RCode("B".to_owned()),
            RdNode::Verb("C".to_owned()),
        ];
        assert_eq!(
            SectionTitleKey::from_rd(&mixed),
            SectionTitleKey::from_rd(&[RdNode::Text("ABC".to_owned())]),
        );
    }

    #[test]
    fn sexpr_remains_structural_against_plain_text() {
        let sexpr = RdNode::tagged(
            RdTag::Sexpr,
            None,
            vec![RdNode::group(vec![RdNode::RCode("A".to_owned())])],
        );
        let noncanonical_sexpr = RdNode::tagged(
            RdTag::Sexpr,
            None,
            vec![RdNode::group(vec![RdNode::Text("A".to_owned())])],
        );
        assert_ne!(
            SectionTitleKey::from_rd(std::slice::from_ref(&sexpr)),
            SectionTitleKey::from_rd(&[RdNode::Text("A".to_owned())]),
        );
        assert_eq!(
            SectionTitleKey::from_rd(&[sexpr]),
            SectionTitleKey::from_rd(&[noncanonical_sexpr]),
        );
    }
}
