//! Static parsing and rendering of the supported `Authors@R` subset.
//!
//! This module deliberately parses the field as an independent R expression.
//! It does not register the field as a source file, and it keeps arity's CST
//! types behind this adapter boundary.

use std::fmt;

use arity_parser::ast::{AstNode, CallExpr, Expr};
use arity_parser::parser::parse;
use arity_parser::syntax::{SyntaxElement, SyntaxKind};

use super::facts::{RNameDecodeError, decode_authors_string_literal};

/// A position-aware failure while parsing an `Authors@R` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorsParseError {
    /// The byte offset in the field where the problem was found.
    pub offset: usize,
    /// A caller-facing explanation of the problem.
    pub message: String,
}

impl AuthorsParseError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for AuthorsParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Authors@R parse error at byte {}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for AuthorsParseError {}

/// One comment entry from a `person()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentSpec {
    /// The R vector name, or `None` for an unnamed entry.
    pub name: Option<String>,
    /// The decoded comment value.
    pub value: String,
}

/// The section in which roxygen2 places a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PersonSection {
    /// A person whose roles contain `cre`.
    Maintainer,
    /// A person whose roles contain `aut` but not `cre`.
    Author,
    /// A person with neither `cre` nor `aut`.
    OtherContributor,
}

/// Owned, parser-independent representation of one supported `person()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonSpec {
    /// The given name, when supplied.
    pub given: Option<String>,
    /// The family name, when supplied.
    pub family: Option<String>,
    /// The middle name, retained by the parser although roxygen2's author
    /// description does not include this person field.
    pub middle: Option<String>,
    /// The email address, when supplied.
    pub email: Option<String>,
    /// The role codes in source order.
    pub role: Vec<String>,
    /// The comment entries in source order.
    pub comment: Vec<CommentSpec>,
}

/// The rendered text and placement metadata for one person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPerson {
    /// The Rd-ready author description, excluding any surrounding section.
    pub description: String,
    /// The primary section selected by roxygen2's role precedence.
    pub section: PersonSection,
    /// Whether this person must also be inserted at the front of the authors
    /// list because both `cre` and `aut` are present.
    pub also_author: bool,
    /// Unknown role codes omitted from `description`, in source order.
    pub unknown_roles: Vec<String>,
}

/// Parses one restricted, statically analyzable `Authors@R` field.
pub fn parse_authors(field: &str) -> Result<Vec<PersonSpec>, AuthorsParseError> {
    if field.trim().is_empty() {
        return Err(AuthorsParseError::new(0, "the field is empty"));
    }

    let parsed = parse(field);
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(AuthorsParseError::new(
            diagnostic.start,
            diagnostic.message.clone(),
        ));
    }

    let mut significant = Vec::new();
    for element in parsed.cst.children_with_tokens() {
        match &element {
            SyntaxElement::Token(token)
                if matches!(
                    token.kind(),
                    SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                ) => {}
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::SEMICOLON => {
                return Err(error_at(
                    &element,
                    "a semicolon is not part of one Authors@R expression",
                ));
            }
            _ => significant.push(element),
        }
    }

    let [element] = significant.as_slice() else {
        let offset = significant
            .first()
            .map(element_offset)
            .unwrap_or(field.len());
        return Err(AuthorsParseError::new(
            offset,
            "the field must contain exactly one expression",
        ));
    };
    let expression = Expr::cast(element.clone())
        .ok_or_else(|| error_at(element, "the top-level value must be c(...) or person(...)"))?;

    match expression {
        Expr::Call(call) if is_simple_call(&call, "person") => {
            parse_person_call(&call, field).map(|person| vec![person])
        }
        Expr::Call(call) if is_simple_call(&call, "c") => parse_people_vector(&call, field),
        _ => Err(error_at(
            element,
            "the top-level value must be c(...) or person(...)",
        )),
    }
}

impl PersonSpec {
    /// Renders this person using roxygen2-compatible package-author formatting
    /// rules. MARC labels use the Library of Congress table; exact label-text
    /// parity with R is not promised.
    #[must_use]
    pub fn render(&self) -> RenderedPerson {
        let mut description = self.given.clone().unwrap_or_default();
        if let Some(family) = &self.family {
            description.push(' ');
            description.push_str(family);
        }
        if let Some(email) = &self.email {
            description.push_str(" \\email{");
            description.push_str(email);
            description.push('}');
        }

        let mut remaining_comments = self.comment.clone();
        append_identity_comment(
            &mut description,
            &mut remaining_comments,
            "ORCID",
            "https://orcid.org/",
            "ORCID",
        );
        append_identity_comment(
            &mut description,
            &mut remaining_comments,
            "ROR",
            "https://ror.org/",
            "ROR",
        );
        if !remaining_comments.is_empty() {
            let rendered = remaining_comments
                .iter()
                .map(|comment| match &comment.name {
                    Some(name) if !name.is_empty() => {
                        format!("{}: {}", name, comment.value)
                    }
                    _ => comment.value.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            description.push_str(" (");
            description.push_str(&rendered);
            description.push(')');
        }

        let mut unknown_roles = Vec::new();
        let extra_roles = self
            .role
            .iter()
            .filter(|role| role.as_str() != "aut" && role.as_str() != "cre")
            .filter_map(|role| match crate::marc_roles::role_term(role) {
                Some(term) => Some(term),
                None => {
                    unknown_roles.push(role.clone());
                    None
                }
            })
            .collect::<Vec<_>>();
        if !extra_roles.is_empty() {
            description.push_str(" [");
            description.push_str(&extra_roles.join(", "));
            description.push(']');
        }

        let has_cre = self.role.iter().any(|role| role == "cre");
        let has_aut = self.role.iter().any(|role| role == "aut");
        let section = if has_cre {
            PersonSection::Maintainer
        } else if has_aut {
            PersonSection::Author
        } else {
            PersonSection::OtherContributor
        };
        RenderedPerson {
            description,
            section,
            also_author: has_cre && has_aut,
            unknown_roles,
        }
    }
}

fn parse_people_vector(call: &CallExpr, field: &str) -> Result<Vec<PersonSpec>, AuthorsParseError> {
    let args = call
        .arg_list()
        .ok_or_else(|| error_at(&call.syntax().clone().into(), "c(...) has no argument list"))?;
    let mut people = Vec::new();
    for arg in args.args() {
        if arg.name().is_some() {
            return Err(error_at(
                &arg.syntax().clone().into(),
                "top-level c(...) entries must be positional person(...) calls",
            ));
        }
        let value = arg.value().ok_or_else(|| {
            error_at(
                &arg.syntax().clone().into(),
                "top-level c(...) cannot contain an empty entry",
            )
        })?;
        let expression = Expr::cast(value.clone())
            .ok_or_else(|| error_at(&value, "each Authors@R entry must be a person(...) call"))?;
        let Expr::Call(person) = expression else {
            return Err(error_at(
                &value,
                "each Authors@R entry must be a person(...) call",
            ));
        };
        if !is_simple_call(&person, "person") {
            return Err(error_at(
                &value,
                "each Authors@R entry must be a person(...) call",
            ));
        }
        people.push(parse_person_call(&person, field)?);
    }
    if people.is_empty() {
        return Err(error_at(
            &call.syntax().clone().into(),
            "c(...) must contain at least one person(...) call",
        ));
    }
    Ok(people)
}

/// How a `person()` formal slot was claimed, so that a genuine duplicate can
/// be told apart from the argument mixing this parser does not support.
#[derive(Clone, Copy)]
enum ArgumentSource {
    Named,
    Positional,
}

/// Parses one supported person() call.
///
/// R matches named arguments before positional ones. Mini-roxygen instead
/// matches arguments in source order with an independent positional counter,
/// so a call that gives given, family, middle, or email by name and also
/// supplies a positional argument that would land on an already-named slot is
/// rejected rather than parsed. This is a deliberate restriction of the
/// supported subset, not a bug; the workaround is to write those four fields
/// as named arguments.
fn parse_person_call(call: &CallExpr, field: &str) -> Result<PersonSpec, AuthorsParseError> {
    let args = call.arg_list().ok_or_else(|| {
        error_at(
            &call.syntax().clone().into(),
            "person(...) has no argument list",
        )
    })?;

    // Report a genuine repeated named formal before source-order matching can
    // misclassify it as positional/named mixing. Unknown names are ignored
    // here so the existing loop remains responsible for their diagnostics.
    let mut named_counts = [0usize; 6];
    for arg in args.args() {
        let Some(name) = arg.name().map(|name| name.to_string()) else {
            continue;
        };
        let Some(slot) = (match name.as_str() {
            "given" => Some(0),
            "family" => Some(1),
            "middle" => Some(2),
            "email" => Some(3),
            "role" => Some(4),
            "comment" => Some(5),
            _ => None,
        }) else {
            continue;
        };
        named_counts[slot] += 1;
        if named_counts[slot] > 1 {
            return Err(error_at(
                &arg.syntax().clone().into(),
                format!("duplicate argument {name:?}"),
            ));
        }
    }

    let mut values: [Option<String>; 4] = [None, None, None, None];
    let mut assigned = [None; 6];
    let mut positional = 0usize;
    let mut roles = Vec::new();
    let mut comments = Vec::new();

    for arg in args.args() {
        let arg_element = arg.syntax().clone().into();
        let (slot, name) = if let Some(name) = arg.name() {
            let name = name.to_string();
            let slot = match name.as_str() {
                "given" => 0,
                "family" => 1,
                "middle" => 2,
                "email" => 3,
                "role" => 4,
                "comment" => 5,
                _ => {
                    return Err(error_at(
                        &arg_element,
                        format!("unknown argument name {name:?}"),
                    ));
                }
            };
            (slot, Some(name))
        } else {
            if positional >= 4 {
                return Err(error_at(
                    &arg_element,
                    "only given, family, middle, and email accept positional arguments",
                ));
            }
            let slot = positional;
            positional += 1;
            (slot, None)
        };

        match (assigned[slot], name.as_deref()) {
            (None, Some(_)) => assigned[slot] = Some(ArgumentSource::Named),
            (None, None) => assigned[slot] = Some(ArgumentSource::Positional),
            // Only a named argument landing on a named slot is the duplicate R
            // itself rejects. Any collision involving a positional argument is
            // one R would have matched, and this parser does not support.
            (Some(ArgumentSource::Named), Some(label)) => {
                return Err(error_at(
                    &arg_element,
                    format!("duplicate argument {label:?}"),
                ));
            }
            (Some(_), _) => {
                return Err(error_at(
                    &arg_element,
                    "person() does not support mixing positional and named name fields; rewrite given, family, middle, and email as named arguments",
                ));
            }
        }

        let value = arg.value();
        if slot < 4 {
            let Some(value) = value else {
                if name.is_some() {
                    return Err(error_at(
                        &arg_element,
                        "a named person field must have a string value",
                    ));
                }
                continue;
            };
            values[slot] = Some(parse_single_string(
                &value,
                field,
                ["given", "family", "middle", "email"][slot],
            )?);
        } else if slot == 4 {
            let value = value
                .ok_or_else(|| error_at(&arg_element, "role must have a string or c(...) value"))?;
            roles = parse_string_vector(&value, field, "role", false)?;
        } else {
            let value = value.ok_or_else(|| {
                error_at(&arg_element, "comment must have a string or c(...) value")
            })?;
            comments = parse_comment_vector(&value, field)?;
        }
    }

    Ok(PersonSpec {
        given: values[0].take(),
        family: values[1].take(),
        middle: values[2].take(),
        email: values[3].take(),
        role: roles,
        comment: comments,
    })
}

fn parse_single_string(
    element: &SyntaxElement,
    _field: &str,
    field_name: &str,
) -> Result<String, AuthorsParseError> {
    let expression = Expr::cast(element.clone()).ok_or_else(|| {
        error_at(
            element,
            format!("{field_name} must be a single string literal"),
        )
    })?;
    let Expr::StringLiteral(literal) = expression else {
        let detail = if matches!(expression, Expr::Call(_)) {
            "must be a single string literal; vectors are not supported"
        } else {
            "must be a string literal"
        };
        return Err(error_at(element, format!("{field_name} {detail}")));
    };
    decode_authors_string_literal(&literal).map_err(|error| {
        let message = match error {
            RNameDecodeError::MixedUnicodeAndByteEscapes => format!(
                "{field_name} mixes Unicode escapes with hex or octal escapes, which R rejects: mixing Unicode and octal/hex escapes in a string is not allowed"
            ),
            RNameDecodeError::NulCharacter => format!(
                "{field_name} contains a nul character, which R rejects: nul character not allowed"
            ),
            RNameDecodeError::EmptyName
            | RNameDecodeError::ContainsBackslash
            | RNameDecodeError::InvalidSpelling => format!(
                r#"{field_name} contains an R string escape mini-roxygen cannot decode; supported escapes are \a \b \f \n \r \t \v \\ \" \' \`, \xHH, \uHHHH, \u{{HHHH}}, \UHHHHHHHH, \U{{HHHHHHHH}}, and octal \NNN"#
            ),
        };
        error_at(
            element,
            message,
        )
    })
}

fn parse_string_vector(
    element: &SyntaxElement,
    field: &str,
    field_name: &str,
    allow_names: bool,
) -> Result<Vec<String>, AuthorsParseError> {
    let expression = Expr::cast(element.clone()).ok_or_else(|| {
        error_at(
            element,
            format!("{field_name} must be a string literal or c(...) of strings"),
        )
    })?;
    match expression {
        Expr::StringLiteral(literal) => decode_authors_string_literal(&literal)
            .map(|value| vec![value])
            .map_err(|_| error_at(element, "role contains an invalid R string escape")),
        Expr::Call(call) if is_simple_call(&call, "c") => {
            let args = call.arg_list().ok_or_else(|| {
                error_at(element, format!("{field_name} c(...) has no argument list"))
            })?;
            let mut values = Vec::new();
            for arg in args.args() {
                if !allow_names && arg.name().is_some() {
                    return Err(error_at(
                        &arg.syntax().clone().into(),
                        format!("{field_name} entries must be unnamed"),
                    ));
                }
                let value = arg.value().ok_or_else(|| {
                    error_at(
                        &arg.syntax().clone().into(),
                        format!("{field_name} c(...) entries must be strings"),
                    )
                })?;
                let value = parse_single_string(&value, field, field_name)?;
                values.push(value);
            }
            if values.is_empty() {
                return Err(error_at(
                    element,
                    format!("{field_name} c(...) must contain at least one string"),
                ));
            }
            Ok(values)
        }
        _ => Err(error_at(
            element,
            format!("{field_name} must be a string literal or c(...) of strings"),
        )),
    }
}

fn parse_comment_vector(
    element: &SyntaxElement,
    field: &str,
) -> Result<Vec<CommentSpec>, AuthorsParseError> {
    let expression = Expr::cast(element.clone()).ok_or_else(|| {
        error_at(
            element,
            "comment must be a string literal or c(...) of strings",
        )
    })?;
    if let Expr::StringLiteral(literal) = expression {
        return decode_authors_string_literal(&literal)
            .map(|value| vec![CommentSpec { name: None, value }])
            .map_err(|_| error_at(element, "comment contains an invalid R string escape"));
    }
    let Expr::Call(call) = expression else {
        return Err(error_at(
            element,
            "comment must be a string literal or c(...) of strings",
        ));
    };
    if !is_simple_call(&call, "c") {
        return Err(error_at(
            element,
            "comment must be a string literal or c(...) of strings",
        ));
    }
    let args = call
        .arg_list()
        .ok_or_else(|| error_at(element, "comment c(...) has no argument list"))?;
    let mut comments = Vec::new();
    for arg in args.args() {
        let value = arg.value().ok_or_else(|| {
            error_at(
                &arg.syntax().clone().into(),
                "comment c(...) entries must be strings",
            )
        })?;
        comments.push(CommentSpec {
            name: arg.name().map(|name| name.to_string()),
            value: parse_single_string(&value, field, "comment")?,
        });
    }
    if comments.is_empty() {
        return Err(error_at(
            element,
            "comment c(...) must contain at least one string",
        ));
    }
    Ok(comments)
}

fn append_identity_comment(
    description: &mut String,
    comments: &mut Vec<CommentSpec>,
    name: &str,
    prefix: &str,
    label: &str,
) {
    let Some(index) = comments
        .iter()
        .position(|comment| comment.name.as_deref() == Some(name))
    else {
        return;
    };
    let value = comments[index].value.clone();
    let href = if value.starts_with("http://") || value.starts_with("https://") {
        value
    } else {
        format!("{prefix}{value}")
    };
    description.push_str(" (\\href{");
    description.push_str(&href);
    description.push_str("}{");
    description.push_str(label);
    description.push_str("})");
    comments.retain(|comment| comment.name.as_deref() != Some(name));
}

fn is_simple_call(call: &CallExpr, name: &str) -> bool {
    matches!(
        call.base(),
        Some(SyntaxElement::Token(token))
            if token.kind() == SyntaxKind::IDENT && token.text() == name
    )
}

fn element_offset(element: &SyntaxElement) -> usize {
    element.text_range().start().into()
}

fn error_at(element: &SyntaxElement, message: impl Into<String>) -> AuthorsParseError {
    AuthorsParseError::new(element_offset(element), message)
}

#[cfg(test)]
mod tests {
    use super::{PersonSection, parse_authors};

    #[test]
    fn reports_unsupported_named_and_positional_mixing() {
        let message = parse_authors(r#"person(given = "First", "Second")"#)
            .unwrap_err()
            .message;
        assert!(
            message.contains("person() does not support mixing positional and named name fields")
        );
        assert!(!message.contains("duplicate"));

        let message = parse_authors(r#"person("First", given = "Second")"#)
            .unwrap_err()
            .message;
        assert!(
            message.contains("person() does not support mixing positional and named name fields")
        );
        assert!(!message.contains("duplicate"));
    }

    #[test]
    fn keeps_true_named_duplicates_as_duplicate_errors() {
        let error = parse_authors(r#"person(given = "First", given = "Second")"#).unwrap_err();
        assert!(error.message.contains("duplicate argument \"given\""));
        assert!(
            !error
                .message
                .contains("mixing positional and named name fields")
        );
    }

    #[test]
    fn explains_the_supported_string_escape_subset() {
        let error = parse_authors(r#"person(given = "\q")"#).unwrap_err();
        assert!(error.message.starts_with(
            "given contains an R string escape mini-roxygen cannot decode; supported escapes are"
        ));
        assert!(
            error
                .message
                .contains(r"\xHH, \uHHHH, \u{HHHH}, \UHHHHHHHH, \U{HHHHHHHH}, and octal \NNN")
        );
    }

    #[test]
    fn reports_r_rejections_for_mixed_escapes_and_nul() {
        let mixed = parse_authors(r#"person(given = "\u41\x42")"#)
            .unwrap_err()
            .message;
        assert_eq!(
            mixed,
            "given mixes Unicode escapes with hex or octal escapes, which R rejects: mixing Unicode and octal/hex escapes in a string is not allowed"
        );

        let nul = parse_authors(r#"person(given = "\u{0}")"#)
            .unwrap_err()
            .message;
        assert_eq!(
            nul,
            "given contains a nul character, which R rejects: nul character not allowed"
        );
    }

    #[test]
    fn pre_scans_named_duplicates_before_mixing_diagnostics() {
        let error =
            parse_authors(r#"person("First", given = "Second", given = "Third")"#).unwrap_err();
        assert_eq!(error.message, "duplicate argument \"given\"");

        let error = parse_authors(r#"person(given = "First", given = "Second")"#).unwrap_err();
        assert_eq!(error.message, "duplicate argument \"given\"");
    }

    #[test]
    fn preserves_supported_argument_orderings() {
        let people = parse_authors(
            r#"person(role = c("aut", "cre"), "GivenOne", "FamilyOne", email = "s@example.com")"#,
        )
        .unwrap();
        assert_eq!(people[0].given.as_deref(), Some("GivenOne"));
        assert_eq!(people[0].family.as_deref(), Some("FamilyOne"));
        assert_eq!(people[0].email.as_deref(), Some("s@example.com"));

        let people =
            parse_authors(r#"person("GivenTwo", "FamilyTwo", role = c("aut", "cre"))"#).unwrap();
        assert_eq!(people[0].given.as_deref(), Some("GivenTwo"));
        assert_eq!(people[0].family.as_deref(), Some("FamilyTwo"));

        let people =
            parse_authors(r#"person(family = "Fixture Organization", role = c("cph", "fnd"))"#)
                .unwrap();
        assert_eq!(people[0].family.as_deref(), Some("Fixture Organization"));
    }

    #[test]
    fn parses_the_common_blank_middle_form() {
        let people =
            parse_authors(r#"person("Given", "Family", , "a@b.c", role = "aut")"#).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].given.as_deref(), Some("Given"));
        assert_eq!(people[0].family.as_deref(), Some("Family"));
        assert_eq!(people[0].middle, None);
        assert_eq!(people[0].email.as_deref(), Some("a@b.c"));
        assert_eq!(people[0].role, ["aut"]);
    }

    #[test]
    fn parses_role_spellings_and_comments() {
        for (source, expected) in [
            (r#"person("A", role = "aut")"#, vec!["aut"]),
            (r#"person("A", role = c("aut", "ctb"))"#, vec!["aut", "ctb"]),
            (r#"person("A", role = c("aut"))"#, vec!["aut"]),
        ] {
            assert_eq!(parse_authors(source).unwrap()[0].role, expected);
        }

        let orcid =
            parse_authors(r#"person("A", comment = c(ORCID = "0000-0000-0000-0000"))"#).unwrap();
        assert_eq!(orcid[0].comment[0].name.as_deref(), Some("ORCID"));

        let ror = parse_authors(r#"person("A", comment = c(ROR = "03wc8by49"))"#).unwrap();
        assert_eq!(
            ror[0].render().description,
            "A (\\href{https://ror.org/03wc8by49}{ROR})"
        );

        let plain = parse_authors(r#"person("A", comment = "embedded library")"#).unwrap();
        assert_eq!(plain[0].render().description, "A (embedded library)");
    }

    #[test]
    fn parses_organisations_named_arguments_and_unicode() {
        let people =
            parse_authors(r#"person(given = "Fixture Organization", role="cph", email = "a@b.c")"#)
                .unwrap();
        assert_eq!(
            people[0].render().description,
            "Fixture Organization \\email{a@b.c} [copyright holder]"
        );
        assert_eq!(people[0].family, None);

        let unicode = parse_authors(r#"person("Unicode", "Fran\u00e7ais")"#).unwrap();
        assert_eq!(unicode[0].render().description, "Unicode Français");
        let literal = parse_authors(r#"person("Literal", "Éclair")"#).unwrap();
        assert_eq!(literal[0].render().description, "Literal Éclair");
    }

    #[test]
    fn renders_sections_roles_and_identity_comments() {
        let people = parse_authors(
            r#"c(
 person("Maintainer", role = c("aut", "cre", "cph"), comment = c(ORCID = "https://orcid.org/id", ROR = "ror-id")),
 person("Author", role = "aut"),
 person("Other", role = c("cph", "fnd"), comment = c(ORCID = "id", note = "x", "unnamed")),
 person("Unknown", role = c("zzz"))
)"#,
        )
        .unwrap();
        let rendered: Vec<_> = people.iter().map(|person| person.render()).collect();
        assert_eq!(rendered[0].section, PersonSection::Maintainer);
        assert!(rendered[0].also_author);
        assert_eq!(
            rendered[0].description,
            "Maintainer (\\href{https://orcid.org/id}{ORCID}) (\\href{https://ror.org/ror-id}{ROR}) [copyright holder]"
        );
        assert_eq!(rendered[1].section, PersonSection::Author);
        assert_eq!(rendered[2].section, PersonSection::OtherContributor);
        assert_eq!(
            rendered[2].description,
            "Other (\\href{https://orcid.org/id}{ORCID}) (note: x, unnamed) [copyright holder, funder]"
        );
        assert_eq!(rendered[3].unknown_roles, ["zzz"]);
        assert_eq!(rendered[3].description, "Unknown");
    }

    #[test]
    fn parses_multiline_and_bare_person_calls() {
        let people = parse_authors(
            r#"person(
"Given",
    "Family",
      ,
 "given@example.org",
role=c("aut", "ctb")
)"#,
        )
        .unwrap();
        assert_eq!(
            people[0].render().description,
            "Given Family \\email{given@example.org} [contributor]"
        );
        assert_eq!(parse_authors(r#"person("Only given")"#).unwrap().len(), 1);
    }

    #[test]
    fn rejects_unsupported_shapes() {
        for source in [
            "person(x)",
            "utils::person(\"A\")",
            "person(given = \"A\", given = \"B\")",
            "person(given = \"A\", nonsense = \"B\")",
            "person(given = c(\"A\", \"B\"))",
            r#""A""#,
            "",
        ] {
            assert!(parse_authors(source).is_err(), "accepted {source:?}");
        }
    }

    fn sections(source: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut maintainer = Vec::new();
        let mut authors = Vec::new();
        let mut other = Vec::new();
        for rendered in parse_authors(source)
            .unwrap()
            .iter()
            .map(|person| person.render())
        {
            match rendered.section {
                PersonSection::Maintainer => maintainer.push(rendered.description.clone()),
                PersonSection::Author => authors.push(rendered.description.clone()),
                PersonSection::OtherContributor => other.push(rendered.description.clone()),
            }
            if rendered.also_author {
                authors.insert(0, rendered.description);
            }
        }
        (maintainer, authors, other)
    }

    #[test]
    fn renders_synthetic_description_fields_end_to_end() {
        let source = r#"c(
    person("Author", "Über", email = "author@example.test", role = "aut"),
    person("Maintainer", "Échantillon", , "maintainer@example.test", role = c("aut", "cre"),
           comment = c(ORCID = "0000-0000-0000-0001")),
    person("Contributor", "Échantillon", role = "ctb", comment = "Documentation notes"),
    person("Fixture Lab", role = c("cph", "fnd"),
           comment = c(ROR = "01fixture01"))
  )"#;
        let (maintainer, authors, other) = sections(source);
        assert_eq!(
            maintainer,
            [
                "Maintainer Échantillon \\email{maintainer@example.test} (\\href{https://orcid.org/0000-0000-0000-0001}{ORCID})"
            ]
        );
        assert_eq!(
            authors,
            [
                "Maintainer Échantillon \\email{maintainer@example.test} (\\href{https://orcid.org/0000-0000-0000-0001}{ORCID})",
                "Author Über \\email{author@example.test}",
            ]
        );
        assert_eq!(
            other,
            [
                "Contributor Échantillon (Documentation notes) [contributor]",
                "Fixture Lab (\\href{https://ror.org/01fixture01}{ROR}) [copyright holder, funder]",
            ]
        );
    }
}
