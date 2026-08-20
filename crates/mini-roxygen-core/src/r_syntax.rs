//! Shared facts about R source syntax.

/// Returns whether R rejects `name` as an unquoted name.
///
/// Quoting more names than strictly required is the safe direction: R accepts
/// a quoted name in a directive, while an unquoted reserved word is parsed as
/// a keyword or constant instead of the requested object.
pub(crate) fn is_reserved_r_word(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "repeat"
            | "while"
            | "function"
            | "for"
            | "in"
            | "next"
            | "break"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "Inf"
            | "NaN"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_complex_"
            | "NA_character_"
    )
}
