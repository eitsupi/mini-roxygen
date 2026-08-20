//! Pins the writer behavior this crate depends on but does not control.
//!
//! R accepts both `r"(...)"` and `r'(...)'` as raw strings, and treats their
//! contents literally. A writer that escapes inside a raw string changes the
//! value R sees, so the escape it inserts becomes part of the documented call.
//! This crate cannot detect that from its own side, which is what makes the
//! behavior worth pinning here rather than leaving it to the dependency.

use rd_ast::{RdDocument, RdNode, RdTag};
use rd_writer::{Writer, WriterOptions};

fn write_usage(code: &str) -> String {
    Writer::new(WriterOptions::default())
        .write_document(&RdDocument::from(vec![RdNode::tagged(
            RdTag::Usage,
            None,
            vec![RdNode::RCode(code.to_owned())],
        )]))
        .expect("the writer accepts a usage section holding R code")
}

#[test]
fn raw_strings_reach_the_output_unescaped_in_both_spellings() {
    for code in [r#"f(x = r"(100%\q)")"#, r"f(x = r'(100%\q)')"] {
        assert!(
            write_usage(code).contains(code),
            "the writer altered the raw string in {code}"
        );
    }
}

#[test]
fn text_outside_a_raw_string_is_still_escaped() {
    let written = write_usage(r"f(x = 100%)");
    assert!(
        written.contains(r"100\%"),
        "a bare percent must stay escaped: {written}"
    );
}
