use crate::r_syntax::is_reserved_r_word;

/// Uses double-quoted R strings for every name that is not a conservative
/// ASCII syntactic name. This follows roxygen2's `auto_quote` convention and
/// makes operators, replacement functions, non-ASCII names, and malformed
/// bare-name positions unambiguous. Quotes and backslashes are escaped here;
/// NUL is rejected because no NAMESPACE source spelling can carry it.
pub(super) fn quote_name(name: &str) -> String {
    if is_syntactic_ascii_name(name) {
        return name.to_owned();
    }
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('"');
    for character in name.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write;
                let _ = write!(quoted, "\\x{:02X}", character as u32);
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn is_syntactic_ascii_name(name: &str) -> bool {
    if is_reserved_r_word(name) {
        return false;
    }
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let first_is_valid = first.is_ascii_alphabetic() || first == '.';
    let dot_digit = first == '.'
        && characters
            .clone()
            .next()
            .is_some_and(|c| c.is_ascii_digit());
    first_is_valid
        && !dot_digit
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '.' || character == '_'
        })
}
