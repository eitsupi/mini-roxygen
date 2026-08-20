//! Decodes R names and string spellings.
//!
//! This boundary is separate because decoding owns the invariant that an RName contains exactly an R-produced value, while the other fact modules only consume that invariant.

use arity_parser::ast::{AstToken, Ident, StringLit};
use arity_parser::syntax::SyntaxKind;

use crate::arity_adapter::raw_string::{is_raw_string_spelling, raw_string_contents};

/// The kind of delimiter used by an R name token.
///
/// This is deliberately not part of the crate's public surface: together with
/// the decoder it would let a caller assemble an [`RName`] from arbitrary
/// text, which is exactly what the decoder exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::arity_adapter) enum RNameDelimiter {
    Bare,
    Backtick,
    Quoted,
    Raw,
}

/// A reason an R name could not be decoded without implementing R's escape
/// grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RNameDecodeError {
    EmptyName,
    ContainsBackslash,
    InvalidSpelling,
    MixedUnicodeAndByteEscapes,
    NulCharacter,
}

/// A name whose value is exactly the text R binds or produces.
///
/// The private field and decoder-only construction are intentional: callers
/// cannot turn arbitrary text into an R name and accidentally bypass decoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RName(String);

impl RName {
    /// Decodes a name token spelling exactly when its value is unambiguous
    /// without implementing R's escape grammar.
    ///
    /// Only this module calls it, and it is the only way to build an
    /// [`RName`]. R rejects a zero-length variable name, so an empty value is
    /// refused here; a string *value* may be empty and uses
    /// [`decode_string_value`] instead.
    pub(in crate::arity_adapter) fn decode(
        spelling: &str,
        delimiter: RNameDelimiter,
    ) -> Result<Self, RNameDecodeError> {
        let value = decode_string_value(spelling, delimiter)?;
        if value.is_empty() {
            return Err(RNameDecodeError::EmptyName);
        }
        Ok(Self(value))
    }

    /// Returns the decoded R name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Decodes a string or name token spelling to the value R produces, exactly
/// when that value is unambiguous without implementing R's escape grammar.
pub(in crate::arity_adapter) fn decode_string_value(
    spelling: &str,
    delimiter: RNameDelimiter,
) -> Result<String, RNameDecodeError> {
    let value = match delimiter {
        RNameDelimiter::Bare => {
            if spelling.contains('\\') {
                return Err(RNameDecodeError::ContainsBackslash);
            }
            spelling
        }
        RNameDelimiter::Backtick | RNameDelimiter::Quoted => {
            let Some((first, last)) = spelling.as_bytes().first().zip(spelling.as_bytes().last())
            else {
                return Err(RNameDecodeError::InvalidSpelling);
            };
            let valid = match delimiter {
                RNameDelimiter::Backtick => spelling.len() >= 2 && *first == b'`' && *last == b'`',
                RNameDelimiter::Quoted => {
                    spelling.len() >= 2 && matches!(*first, b'"' | b'\'') && first == last
                }
                _ => unreachable!(),
            };
            if !valid {
                return Err(RNameDecodeError::InvalidSpelling);
            }
            let value = &spelling[1..spelling.len() - 1];
            if value.contains('\\') {
                return Err(RNameDecodeError::ContainsBackslash);
            }
            value
        }
        RNameDelimiter::Raw => {
            raw_string_contents(spelling).ok_or(RNameDecodeError::InvalidSpelling)?
        }
    };
    Ok(value.to_owned())
}

/// Classifies a token that could spell an R name, and rejects one that
/// cannot be a name at all.
///
/// Every path that builds an [`RName`] goes through here, so a spelling that
/// is not a name has one place to be caught rather than one per caller.
/// Rejected here are a bare reserved constant, which R does not bind a name
/// through, and an identifier carrying an unbalanced backtick, which the
/// parser produces from unterminated source. The parser also admits a leading
/// underscore that R does not read as a name, and R treats a dot followed by
/// an ASCII digit as numeric syntax rather than a name; those bare spellings
/// are refused for the same reason. This is deliberately not an ASCII name
/// check: R accepts Unicode letters, and their locale-dependent classification
/// does not belong in this adapter.
pub(super) fn name_delimiter(token: &arity_parser::syntax::SyntaxToken) -> Option<RNameDelimiter> {
    match token.kind() {
        SyntaxKind::IDENT => {
            let text = token.text();
            if text.starts_with('`') || text.ends_with('`') {
                (text.len() >= 2 && text.starts_with('`') && text.ends_with('`'))
                    .then_some(RNameDelimiter::Backtick)
            } else if bare_name_is_rejected(text)
                || Ident::cast(token.clone())
                    .is_some_and(|identifier| identifier.is_reserved_constant())
            {
                None
            } else {
                Some(RNameDelimiter::Bare)
            }
        }
        SyntaxKind::STRING => {
            let text = token.text();
            if is_raw_string_spelling(text) {
                Some(RNameDelimiter::Raw)
            } else {
                StringLit::cast(token.clone())?.quote()?;
                Some(RNameDelimiter::Quoted)
            }
        }
        _ => None,
    }
}
pub(super) fn bare_name_is_rejected(text: &str) -> bool {
    text.starts_with('_')
        || (text.as_bytes().first() == Some(&b'.')
            && text
                .as_bytes()
                .get(1)
                .is_some_and(|byte| byte.is_ascii_digit()))
}
pub(super) fn decode_string_literal(literal: &StringLit) -> Result<String, RNameDecodeError> {
    let delimiter = name_delimiter(literal.syntax()).ok_or(RNameDecodeError::InvalidSpelling)?;
    decode_string_value(literal.text(), delimiter)
}

/// Decodes a string literal used by the restricted `Authors@R` grammar.
///
/// The general name decoder intentionally refuses escapes because its callers
/// need a conservative answer about source-level names. Authors are values,
/// however, and R's string escapes are part of their meaning. Keep the shared
/// delimiter and raw-string handling, and add the value decoder only here.
///
/// The accepted escape grammar matches R, but \x and octal escapes still map
/// their values through char::from_u32 as Unicode code points. R produces raw
/// bytes for those escapes instead, so "\xe9" yields é here and an
/// invalid-UTF-8 byte in R. This is a deliberate, accepted divergence in the
/// supported Authors@R subset. Unicode escapes cannot be mixed with hex or
/// octal escapes, and numeric escapes producing NUL are rejected, because R
/// rejects both forms while parsing a string literal.
pub(crate) fn decode_authors_string_literal(
    literal: &StringLit,
) -> Result<String, RNameDecodeError> {
    let delimiter = name_delimiter(literal.syntax()).ok_or(RNameDecodeError::InvalidSpelling)?;
    if delimiter == RNameDelimiter::Raw {
        return decode_string_value(literal.text(), delimiter);
    }
    let inner = literal.inner().ok_or(RNameDecodeError::InvalidSpelling)?;
    if !inner.contains('\\') {
        return decode_string_value(literal.text(), delimiter);
    }
    decode_r_escaped_string(inner)
}

fn decode_r_escaped_string(inner: &str) -> Result<String, RNameDecodeError> {
    let chars: Vec<char> = inner.chars().collect();
    let mut result = String::with_capacity(inner.len());
    let mut index = 0;
    let mut unicode_escape_seen = false;
    let mut byte_escape_seen = false;

    while index < chars.len() {
        let character = chars[index];
        index += 1;
        if character != '\\' {
            result.push(character);
            continue;
        }
        let Some(&escape) = chars.get(index) else {
            return Err(RNameDecodeError::InvalidSpelling);
        };
        index += 1;
        let decoded = match escape {
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0c',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0b',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '`' => '`',
            'x' => {
                if unicode_escape_seen {
                    return Err(RNameDecodeError::MixedUnicodeAndByteEscapes);
                }
                byte_escape_seen = true;
                let value = hex_escape(&chars, &mut index, 2)?;
                decode_numeric_escape(value)?
            }
            'u' => {
                if byte_escape_seen {
                    return Err(RNameDecodeError::MixedUnicodeAndByteEscapes);
                }
                unicode_escape_seen = true;
                let value = if chars.get(index) == Some(&'{') {
                    braced_hex_escape(&chars, &mut index, 4)?
                } else {
                    hex_escape(&chars, &mut index, 4)?
                };
                decode_numeric_escape(value)?
            }
            'U' => {
                if byte_escape_seen {
                    return Err(RNameDecodeError::MixedUnicodeAndByteEscapes);
                }
                unicode_escape_seen = true;
                let value = if chars.get(index) == Some(&'{') {
                    braced_hex_escape(&chars, &mut index, 8)?
                } else {
                    hex_escape(&chars, &mut index, 8)?
                };
                decode_numeric_escape(value)?
            }
            '0'..='7' => {
                if unicode_escape_seen {
                    return Err(RNameDecodeError::MixedUnicodeAndByteEscapes);
                }
                byte_escape_seen = true;
                let mut value = (escape as u32) - ('0' as u32);
                let mut digits = 1;
                while digits < 3 {
                    let Some(next) = chars.get(index).copied() else {
                        break;
                    };
                    if !matches!(next, '0'..='7') {
                        break;
                    }
                    value = value * 8 + (next as u32) - ('0' as u32);
                    index += 1;
                    digits += 1;
                }
                decode_numeric_escape(value)?
            }
            _ => return Err(RNameDecodeError::InvalidSpelling),
        };
        result.push(decoded);
    }

    Ok(result)
}

fn decode_numeric_escape(value: u32) -> Result<char, RNameDecodeError> {
    if value == 0 {
        return Err(RNameDecodeError::NulCharacter);
    }
    char::from_u32(value).ok_or(RNameDecodeError::InvalidSpelling)
}

fn hex_escape(chars: &[char], index: &mut usize, max: usize) -> Result<u32, RNameDecodeError> {
    let start = *index;
    while *index < chars.len() && *index - start < max && chars[*index].is_ascii_hexdigit() {
        *index += 1;
    }
    if *index == start {
        return Err(RNameDecodeError::InvalidSpelling);
    }
    parse_hex_digits(&chars[start..*index])
}

fn braced_hex_escape(
    chars: &[char],
    index: &mut usize,
    max: usize,
) -> Result<u32, RNameDecodeError> {
    *index += 1;
    let value = hex_escape(chars, index, max)?;
    if chars.get(*index) != Some(&'}') {
        return Err(RNameDecodeError::InvalidSpelling);
    }
    *index += 1;
    Ok(value)
}

fn parse_hex_digits(digits: &[char]) -> Result<u32, RNameDecodeError> {
    digits.iter().try_fold(0u32, |value, digit| {
        let digit = digit
            .to_digit(16)
            .ok_or(RNameDecodeError::InvalidSpelling)?;
        let value = value
            .checked_mul(16)
            .and_then(|value| value.checked_add(digit))
            .ok_or(RNameDecodeError::InvalidSpelling)?;
        Ok(value)
    })
}

#[cfg(test)]
mod tests {
    use crate::arity_adapter::test_support::{function, parsed};
    use crate::arity_adapter::{AssignmentTarget, FormalError, RNameDecodeError, TopLevelShape};

    #[test]
    fn decodes_authors_r_escape_widths_and_braced_forms() {
        for (spelling, expected) in [
            (r"\xA", "\u{a}"),
            (r"\x41", "A"),
            (r"\u41", "A"),
            (r"\u00e9", "é"),
            (r"\u{e9}", "é"),
            (r"\U41", "A"),
            (r"\U0001F600", "😀"),
            (r"\U{1F600}", "😀"),
            (r"\u12345", "ሴ5"),
            (r"\xABC", "«C"),
        ] {
            assert_eq!(
                super::decode_r_escaped_string(spelling).as_deref(),
                Ok(expected),
                "{spelling}"
            );
        }
    }

    #[test]
    fn rejects_invalid_authors_r_hex_escapes() {
        for spelling in [
            r"\x",
            r"\u",
            r"\u{}",
            r"\u{41",
            r"\u{111111111}",
            r"\U{110000}",
        ] {
            assert_eq!(
                super::decode_r_escaped_string(spelling),
                Err(RNameDecodeError::InvalidSpelling),
                "{spelling}"
            );
        }
    }

    #[test]
    fn rejects_mixed_unicode_and_byte_escape_families() {
        for spelling in [r"\u41\x42", r"\x41\u42", r"\u41\101", r"\101\u41"] {
            assert_eq!(
                super::decode_r_escaped_string(spelling),
                Err(RNameDecodeError::MixedUnicodeAndByteEscapes),
                "{spelling}"
            );
        }
    }

    #[test]
    fn keeps_hex_and_octal_escapes_compatible() {
        assert_eq!(
            super::decode_r_escaped_string(r"\x41\101").as_deref(),
            Ok("AA")
        );
    }

    #[test]
    fn keeps_single_escape_families_decodable() {
        assert_eq!(
            super::decode_r_escaped_string(r"\u41\u{42}\U43\U{44}").as_deref(),
            Ok("ABCD")
        );
        assert_eq!(
            super::decode_r_escaped_string(r"\x41\x42").as_deref(),
            Ok("AB")
        );
    }

    #[test]
    fn rejects_nul_from_every_numeric_escape_form() {
        for spelling in [r"\0", r"\x0", r"\u0", r"\u{0}", r"\U0", r"\U{0}"] {
            assert_eq!(
                super::decode_r_escaped_string(spelling),
                Err(RNameDecodeError::NulCharacter),
                "{spelling}"
            );
        }
    }

    #[test]
    fn decodes_nonzero_values_from_every_numeric_escape_form() {
        for spelling in [r"\101", r"\x41", r"\u41", r"\u{41}", r"\U41", r"\U{41}"] {
            assert_eq!(
                super::decode_r_escaped_string(spelling).as_deref(),
                Ok("A"),
                "{spelling}"
            );
        }
    }

    #[test]
    fn refuses_escapes_in_names_instead_of_stripping_only_delimiters() {
        let cases = [
            ("`a\\`b` <- 1", RNameDecodeError::ContainsBackslash),
            ("\"a\\\"b\" <- 1", RNameDecodeError::ContainsBackslash),
            ("\"foo\\x2ebar\" <- 1", RNameDecodeError::ContainsBackslash),
        ];
        for (source_text, reason) in cases {
            let (parsed, source) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentTarget::Undecodable {
                span,
                reason: actual,
            } = &fact.target
            else {
                panic!("expected undecodable target for {source_text}");
            };
            // These values are what R 4.x binds; decoding refuses until the
            // complete R escape grammar is implemented.
            assert_eq!(*actual, reason, "{source_text}");
            assert_eq!(
                source.text_range(span.range),
                Some(source_text.split(" <- ").next().unwrap())
            );
        }
    }

    #[test]
    fn decodes_raw_string_names_and_literals_without_escape_processing() {
        let cases = [
            ("r\"(foo)\" <- 1", "foo"),
            ("R'[foo]' <- 1", "foo"),
            ("r\"---{foo}---\" <- 1", "foo"),
            ("r\"{.*\\s*}\" <- 1", ".*\\s*"),
        ];
        for (source_text, expected) in cases {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentTarget::Binding(binding) = &fact.target else {
                panic!("expected decoded raw target for {source_text}");
            };
            // These values are the names R 4.x binds from raw string targets.
            assert_eq!(binding.canonical.as_str(), expected);
        }

        let (parsed, _) = parsed(
            r#"r"(_PACKAGE)"
"#,
        );
        let TopLevelShape::StringLiteral(value) = &parsed.top_level[0].fact.shape else {
            panic!("expected raw string literal");
        };
        // R 4.x produces `_PACKAGE` from this raw string literal.
        assert_eq!(value.value.as_ref().unwrap().as_str(), "_PACKAGE");
    }

    #[test]
    fn an_unbalanced_backtick_is_not_a_name() {
        // The parser produces this identifier from unterminated source. Taking
        // it for a bare name would put a backtick, and the rest of the line,
        // inside a type that promises to be exactly what R binds.
        for source_text in ["1 -> `oops", "`oops <- 1"] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let target = match &parsed.top_level.first().map(|entry| &entry.fact.shape) {
                Some(TopLevelShape::Assignment(fact)) => Some(&fact.target),
                _ => None,
            };
            assert!(
                !matches!(target, Some(AssignmentTarget::Binding(_))),
                "expected no binding for {source_text}, got {target:?}"
            );
        }
    }

    #[test]
    fn a_bare_constant_is_not_a_callee_name() {
        // NULL() parses, but the callee is a constant expression rather than a
        // name, so there is no name to report.
        for source_text in ["NULL()", "TRUE()", "NA()"] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Call(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected a call for {source_text}");
            };
            assert!(
                fact.callee.is_none(),
                "expected no callee name for {source_text}, got {:?}",
                fact.callee
            );
        }
    }

    #[test]
    fn bare_reserved_constants_and_numbers_are_invalid_targets() {
        // Measured with R 4.x: each of these fails with "invalid (do_set)
        // left-hand side to assignment", so none of them binds a name.
        for source_text in [
            "NULL <- 1",
            "TRUE <- 1",
            "FALSE <- 1",
            "NA <- 1",
            "NA_integer_ <- 1",
            "NA_real_ <- 1",
            "NA_complex_ <- 1",
            "NA_character_ <- 1",
            "NaN <- 1",
            "Inf <- 1",
            "1 <- 2",
            "1.5 <- 2",
        ] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            assert!(
                matches!(fact.target, AssignmentTarget::Invalid { .. }),
                "expected an invalid target for {source_text}, got {:?}",
                fact.target
            );
        }
    }

    #[test]
    fn rejects_r_non_names_and_preserves_bare_name_spellings() {
        for spelling in ["_a", "_"] {
            let (parsed_file, _) = parsed(&format!(
                r#"{spelling} <- 1
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed_file.top_level[0].fact.shape else {
                panic!("expected assignment for {spelling}");
            };
            assert!(matches!(fact.target, AssignmentTarget::Invalid { .. }));

            let (parsed_file, _) = parsed(&format!(
                r#"f <- function({spelling}) 1
"#
            ));
            assert_eq!(
                function(&parsed_file).formals,
                Err(FormalError::InvalidStructure)
            );
        }

        for (source_text, expected) in [("`_a` <- 1", "_a"), ("f <- function(`_a`) 1", "_a")] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            if source_text.starts_with('f') {
                assert_eq!(
                    function(&parsed).formals.as_ref().unwrap()[0]
                        .name
                        .value
                        .as_ref()
                        .unwrap()
                        .as_str(),
                    expected
                );
            } else {
                let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                    panic!("expected assignment for {source_text}");
                };
                let AssignmentTarget::Binding(binding) = &fact.target else {
                    panic!("expected binding for {source_text}");
                };
                assert_eq!(binding.canonical.as_str(), expected);
            }
        }

        for spelling in ["._a", ".a", "..", "..1", "...", "a_b", "a_", "a1", ".__x"] {
            let (parsed_file, _) = parsed(&format!(
                r#"{spelling} <- 1
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed_file.top_level[0].fact.shape else {
                panic!("expected assignment for {spelling}");
            };
            let AssignmentTarget::Binding(binding) = &fact.target else {
                panic!("expected binding for {spelling}, got {:?}", fact.target);
            };
            assert_eq!(binding.canonical.as_str(), spelling);

            let (parsed_file, _) = parsed(&format!(
                r#"f <- function({spelling}) 1
"#
            ));
            let formal = &function(&parsed_file).formals.as_ref().unwrap()[0];
            assert_eq!(formal.name.value.as_ref().unwrap().as_str(), spelling);
        }

        // The bare-name rule must not impose an ASCII-only restriction: R
        // reads a Unicode letter as a name, and arity now lexes it as one
        // identifier token.
        assert!(!super::bare_name_is_rejected("日本語"));
        assert_eq!(
            super::RName::decode("日本語", super::RNameDelimiter::Bare)
                .unwrap()
                .as_str(),
            "日本語"
        );
    }

    #[test]
    fn arity_splits_dot_digit_name_like_spelling_into_float_and_ident() {
        let output = arity_parser::parser::parse(
            r#".1a <- 1
"#,
        );
        assert_eq!(output.diagnostics.len(), 1);
        let tokens: Vec<_> = output
            .cst
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .map(|token| (token.kind(), token.text().to_owned()))
            .collect();
        assert_eq!(
            &tokens[..2],
            &[
                (arity_parser::syntax::SyntaxKind::FLOAT, ".1".to_owned()),
                (arity_parser::syntax::SyntaxKind::IDENT, "a".to_owned()),
            ]
        );
    }

    #[test]
    fn quoted_constants_and_rebindable_symbols_are_ordinary_bindings() {
        // Measured with R 4.x: `NULL` <- 1 and T <- 1 both bind, because the
        // delimiter makes the first an ordinary name and T is rebindable.
        let cases = [
            ("`NULL` <- 1", "NULL"),
            ("`TRUE` <- 1", "TRUE"),
            ("\"NA\" <- 1", "NA"),
            ("T <- 1", "T"),
            ("F <- 1", "F"),
        ];
        for (source_text, expected) in cases {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentTarget::Binding(binding) = &fact.target else {
                panic!(
                    "expected a binding for {source_text}, got {:?}",
                    fact.target
                );
            };
            assert_eq!(binding.canonical.as_str(), expected);
        }
    }

    #[test]
    fn an_empty_string_is_a_valid_value_but_not_a_valid_name() {
        // R distinguishes these: "" is a zero-length string value, while
        // "" <- 1 fails with "attempt to use zero-length variable name".
        let (value_file, _) = parsed(
            r#"""
"#,
        );
        let TopLevelShape::StringLiteral(literal) = &value_file.top_level[0].fact.shape else {
            panic!("expected a string literal");
        };
        assert_eq!(literal.value.as_deref(), Ok(""));

        let (target_file, _) = parsed(
            r#""" <- 1
"#,
        );
        let TopLevelShape::Assignment(fact) = &target_file.top_level[0].fact.shape else {
            panic!("expected an assignment");
        };
        // R rejects this outright, so it is invalid source rather than a name
        // awaiting escape decoding.
        assert!(matches!(fact.target, AssignmentTarget::Invalid { .. }));
    }

    #[test]
    fn a_raw_string_closed_early_is_refused() {
        // R ends a raw string at the first closing sequence, so this spelling
        // does not describe one value; accepting the final suffix would report
        // a value R never produces.
        assert_eq!(
            super::decode_string_value(r#"r"(a)"junk)""#, super::RNameDelimiter::Raw),
            Err(RNameDecodeError::InvalidSpelling)
        );
        // The same closer appearing only at the end is fine.
        assert_eq!(
            super::decode_string_value(r#"r"(a)b)""#, super::RNameDelimiter::Raw).as_deref(),
            Ok("a)b")
        );
    }

    #[test]
    fn decodes_all_escape_free_quoted_name_forms() {
        for (source_text, expected) in [
            ("`[.myclass` <- 1", "[.myclass"),
            ("`foo<-` <- 1", "foo<-"),
            ("`%+%` <- 1", "%+%"),
            ("`foo bar` <- 1", "foo bar"),
            ("\"foo\" <- 1", "foo"),
            ("'foo' <- 1", "foo"),
        ] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            let AssignmentTarget::Binding(binding) = &fact.target else {
                panic!("expected decoded target for {source_text}");
            };
            // These values are the names R 4.x binds from escape-free quoted targets.
            assert_eq!(binding.canonical.as_str(), expected);
        }
    }

    #[test]
    fn refuses_zero_length_names_and_distinguishes_compound_targets() {
        for source_text in ["`` <- 1", "\"\" <- 1"] {
            let (parsed, _) = parsed(&format!(
                r#"{source_text}
"#
            ));
            let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
                panic!("expected assignment for {source_text}");
            };
            // R 4.x rejects these zero-length variable names outright, so
            // they are invalid source rather than names awaiting decoding.
            assert!(
                matches!(fact.target, AssignmentTarget::Invalid { .. }),
                "expected an invalid target for {source_text}, got {:?}",
                fact.target
            );
        }
        let (parsed, _) = parsed(
            r#"x[[1]] <- 1
"#,
        );
        let TopLevelShape::Assignment(fact) = &parsed.top_level[0].fact.shape else {
            panic!("expected assignment");
        };
        assert!(matches!(fact.target, AssignmentTarget::Compound { .. }));
    }

    #[test]
    fn recognizes_package_sentinel_from_normal_and_raw_strings() {
        let (parsed, _) = parsed(
            r#""_PACKAGE"
r"(_PACKAGE)"
"#,
        );
        for entry in &parsed.top_level {
            let fact = &entry.fact;
            let TopLevelShape::StringLiteral(value) = &fact.shape else {
                panic!("expected string literal");
            };
            assert_eq!(value.value.as_ref().unwrap().as_str(), "_PACKAGE");
        }
    }
}
