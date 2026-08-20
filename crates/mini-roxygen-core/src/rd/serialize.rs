//! Serialization and conversion of writer failures into diagnostics.

use rd_ast::RdDocument;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Label, Severity};
use crate::source::Span;

use super::origins::{OriginMap, span_for_path};

pub(crate) fn serialize(
    document: &RdDocument,
    origins: &OriginMap,
    anchor: Span,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    match rd_writer::write_document(document) {
        Ok(content) => Some(content),
        Err(error) => {
            let span = error
                .ast_path()
                .and_then(|path| span_for_path(origins, path))
                .unwrap_or(anchor);
            diagnostics.push(Diagnostic::new(
                Severity::Error,
                DiagnosticCode::RdSerializationFailed,
                format!("could not serialize Rd document: {error}"),
                Label::new(span, "Rd serialization failed"),
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use rd_ast::RdNode;
    use rd_writer::WriteError;

    use crate::diagnostic::DiagnosticCode;
    use crate::source::{FileId, Span, TextRange};

    use super::super::origins::OriginBuilder;
    use super::serialize;

    #[test]
    fn writer_error_resolves_to_the_fragment_origin_span() {
        let span = Span::new(FileId::new(0), TextRange::new(9, 10));
        let mut builder = OriginBuilder::new();
        let node = builder.append_node(RdNode::group(Vec::new()));
        builder.record(node, &[span]);
        let (document, origins) = builder.materialize();
        let mut diagnostics = crate::diagnostic::Diagnostics::new();
        assert!(serialize(&document, &origins, span, &mut diagnostics).is_none());
        let diagnostic = diagnostics.iter().next().expect("writer diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::RdSerializationFailed);
        assert_eq!(diagnostic.primary.span, span);
    }

    #[test]
    fn upstream_errors_without_ast_paths_have_no_ast_path() {
        assert!(
            WriteError::Verification {
                reason: "round-trip mismatch".into()
            }
            .ast_path()
            .is_none()
        );
        assert!(
            WriteError::Io {
                source: std::io::Error::other("sink")
            }
            .ast_path()
            .is_none()
        );
    }
}
