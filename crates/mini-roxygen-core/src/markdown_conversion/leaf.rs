//! Physical-line chunking for converted leaf values.

/// Splits `value` at retained physical newlines without changing its bytes.
///
/// This borrows the input, allocates nothing, and rewrites nothing. Empty
/// input yields zero chunks; no trailing newline is appended; every chunk is
/// non-empty and contains at most one newline, only as its last byte; every
/// chunk except possibly the last ends with a newline; and concatenating all
/// chunks reproduces the input byte for byte.
pub(crate) fn physical_line_chunks(value: &str) -> impl Iterator<Item = &str> + '_ {
    value.split_inclusive('\n')
}

#[cfg(test)]
mod tests {
    use super::physical_line_chunks;

    #[test]
    fn physical_line_chunks_follow_the_contract_table() {
        let cases = [
            ("", vec![]),
            ("one", vec!["one"]),
            ("one\n", vec!["one\n"]),
            ("one\n\ntwo", vec!["one\n", "\n", "two"]),
            ("\n\n", vec!["\n", "\n"]),
        ];
        for (input, expected) in cases {
            let actual = physical_line_chunks(input).collect::<Vec<_>>();
            assert_eq!(actual, expected, "input {input:?}");
            assert_eq!(actual.concat(), input, "input {input:?}");
        }
    }
}
