//! Recognition of complete R raw-string spellings.

/// Returns whether `text` is a complete R raw-string spelling accepted by the
/// whole-token recognizer, rather than a prefix or a recovered unterminated
/// token.
pub(crate) fn is_raw_string_spelling(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 4
        && matches!(bytes[0], b'r' | b'R')
        && matches!(bytes[1], b'"' | b'\'')
        && raw_string_contents(text).is_some()
}

/// Returns the contents of a complete R raw-string spelling.
pub(crate) fn raw_string_contents(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 || !matches!(bytes[0], b'r' | b'R') {
        return None;
    }
    let quote = bytes[1];
    if !matches!(quote, b'"' | b'\'') {
        return None;
    }
    let mut bracket_index = 2;
    while bytes.get(bracket_index) == Some(&b'-') {
        bracket_index += 1;
    }
    let close_bracket = match bytes.get(bracket_index) {
        Some(b'(') => b')',
        Some(b'[') => b']',
        Some(b'{') => b'}',
        _ => return None,
    };
    let dash_count = bracket_index - 2;
    let suffix_len = 1 + dash_count + 1;
    if bytes.len() < bracket_index + 1 + suffix_len {
        return None;
    }
    let close_start = bytes.len() - suffix_len;
    if bytes[close_start] != close_bracket
        || bytes[close_start + 1..close_start + 1 + dash_count]
            .iter()
            .any(|byte| *byte != b'-')
        || bytes[bytes.len() - 1] != quote
    {
        return None;
    }
    let content_start = bracket_index + 1;
    if content_start > close_start {
        return None;
    }
    // R ends a raw string at the first closing sequence. Checking only the
    // suffix would accept a spelling whose content already closes it and
    // report a value R never produces.
    let closer = &text[close_start..];
    (!text[content_start..close_start].contains(closer)).then(|| &text[content_start..close_start])
}
