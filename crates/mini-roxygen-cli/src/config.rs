//! Loading and validating the CLI's TOML configuration medium.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mini_roxygen_core::{S3RegistrarRole, S3RegistrarSet, S3RegistrarSignature};

pub(crate) const CONFIG_FILE: &str = "mini-roxygen.toml";

/// TOML values extracted from one CLI configuration file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LoadedConfig {
    pub(crate) entries: BTreeMap<String, String>,
    pub(crate) registrars: S3RegistrarSet,
    pub(crate) origin: String,
}

/// An operational failure while reading or parsing the CLI configuration.
#[derive(Debug)]
pub(crate) enum ConfigError {
    Io { path: PathBuf, message: String },
    Malformed { path: PathBuf, message: String },
}

impl ConfigError {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. } | Self::Malformed { path, .. } => path,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { message, .. } | Self::Malformed { message, .. } => {
                formatter.write_str(message)
            }
        }
    }
}

/// Reads the package-root configuration if present and schema-checks it.
pub(crate) fn load(root: &Path) -> Result<Option<LoadedConfig>, ConfigError> {
    let path = root.join(CONFIG_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Err(ConfigError::Io {
                path,
                message: "could not read configuration as UTF-8".to_owned(),
            });
        }
        Err(error) => {
            return Err(ConfigError::Io {
                path,
                message: format!("could not read configuration: {error}"),
            });
        }
    };
    let (entries, registrars) = parse_config(&text).map_err(|message| ConfigError::Malformed {
        path: path.clone(),
        message,
    })?;
    Ok(Some(LoadedConfig {
        entries,
        registrars,
        origin: path.display().to_string(),
    }))
}

fn parse_config(text: &str) -> Result<(BTreeMap<String, String>, S3RegistrarSet), String> {
    let document = text
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    for (name, value) in &document {
        if name != "inline-r" && name != "s3" {
            let kind = if value.is_table() { "table" } else { "field" };
            return Err(format!("unknown {kind} {name:?}"));
        }
    }
    let mut entries = BTreeMap::new();
    if let Some(inline_r) = document.get("inline-r") {
        let Some(inline_r) = inline_r.as_table() else {
            return Err("unknown field \"inline-r\"".to_owned());
        };
        let Some(substitutions) = inline_r.get("substitutions") else {
            return Err("unknown table \"[inline-r]\"".to_owned());
        };
        let Some(substitutions) = substitutions.as_table() else {
            return Err("unknown field \"substitutions\"".to_owned());
        };
        for (name, value) in inline_r {
            if name != "substitutions" {
                let kind = if value.is_table() { "table" } else { "field" };
                return Err(format!("unknown {kind} \"{name}\""));
            }
        }
        for (key, value) in substitutions {
            let Some(value) = value.as_str() else {
                return Err(format!(
                    "substitution values must be quoted strings: {key:?}"
                ));
            };
            entries.insert(key.clone(), value.to_owned());
        }
    }
    let mut additions = Vec::new();
    if let Some(s3) = document.get("s3") {
        let Some(s3) = s3.as_table() else {
            return Err("unknown field \"s3\"".to_owned());
        };
        let Some(registrars) = s3.get("registrars") else {
            return Err("unknown table \"[s3]\"".to_owned());
        };
        let Some(registrars) = registrars.as_array() else {
            return Err("s3.registrars must be an array of tables".to_owned());
        };
        for registrar in registrars {
            let Some(registrar) = registrar.as_table() else {
                return Err("s3.registrars entries must be tables".to_owned());
            };
            let Some(function) = registrar.get("function").and_then(toml::Value::as_str) else {
                return Err("s3 registrar function must be a quoted string".to_owned());
            };
            let Some(arguments) = registrar.get("arguments").and_then(toml::Value::as_array) else {
                return Err("s3 registrar arguments must be an array of strings".to_owned());
            };
            let mut roles = Vec::new();
            for argument in arguments {
                let Some(argument) = argument.as_str() else {
                    return Err("s3 registrar arguments must be strings".to_owned());
                };
                let role = match argument {
                    "generic" => S3RegistrarRole::Generic,
                    "class" => S3RegistrarRole::Class,
                    "method" => S3RegistrarRole::Method,
                    _ => return Err(format!("unknown s3 registrar argument {argument:?}")),
                };
                roles.push(role);
            }
            additions.push(S3RegistrarSignature::new(function, roles)?);
            for name in registrar.keys() {
                if name != "function" && name != "arguments" {
                    return Err(format!("unknown s3 registrar field {name:?}"));
                }
            }
        }
        for name in s3.keys() {
            if name != "registrars" {
                return Err(format!("unknown field in [s3] {name:?}"));
            }
        }
    }
    let registrars = S3RegistrarSet::with_additions(additions)?;
    Ok((entries, registrars))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{CONFIG_FILE, ConfigError, load};

    fn write_config(body: &str) -> tempfile::TempDir {
        let root = tempdir().expect("temporary configuration directory");
        fs::write(root.path().join(CONFIG_FILE), body).expect("configuration should be writable");
        root
    }

    #[test]
    fn absent_configuration_is_not_an_error() {
        let root = tempdir().expect("temporary configuration directory");
        assert_eq!(
            load(root.path()).expect("missing config should be optional"),
            None
        );
    }

    #[test]
    fn unknown_configuration_fields_are_rejected() {
        let root = write_config("[inline-r]\nunknown = 'value'\n");
        let error = load(root.path()).expect_err("unknown fields must be rejected");
        assert!(error.to_string().contains("unknown table \"[inline-r]\""));
        assert!(error.path().ends_with(CONFIG_FILE));
    }

    #[test]
    fn unknown_top_level_fields_are_rejected() {
        let root = write_config("other = 'value'\n");
        let error = load(root.path()).expect_err("unknown top-level fields must be rejected");
        assert!(error.to_string().contains("unknown field"));
        assert!(error.path().ends_with(CONFIG_FILE));
    }

    #[test]
    fn non_string_substitution_values_are_rejected() {
        let root = write_config("[inline-r.substitutions]\n'number()' = 42\n");
        let error = load(root.path()).expect_err("non-string values must be rejected");
        assert!(error.to_string().contains("quoted strings"));
        assert!(error.path().ends_with(CONFIG_FILE));
    }

    #[test]
    fn duplicate_substitution_keys_are_rejected_by_toml() {
        let root = write_config(
            "[inline-r.substitutions]\n'duplicate()' = '\\code{one}'\n'duplicate()' = '\\code{two}'\n",
        );
        let error = load(root.path()).expect_err("duplicate keys must be rejected");
        assert!(error.to_string().contains("duplicate key"));
        assert!(error.path().ends_with(CONFIG_FILE));
    }

    #[test]
    fn malformed_toml_reports_a_line_or_column() {
        let root = write_config("[inline-r.substitutions\n'broken()' = '\\code{broken}'\n");
        let error = load(root.path()).expect_err("malformed TOML must be rejected");
        let message = error.to_string();
        assert!(message.contains("line") || message.contains("column"));
        assert!(error.path().ends_with(CONFIG_FILE));
    }

    #[test]
    fn configuration_entries_keep_toml_string_values_and_origin() {
        let root = write_config("[inline-r.substitutions]\n'custom()' = '\\code{custom}'\n");
        let loaded = load(root.path())
            .expect("configuration should load")
            .expect("config");
        assert_eq!(loaded.entries["custom()"], r#"\code{custom}"#);
        assert!(loaded.origin.ends_with(CONFIG_FILE));
    }

    #[test]
    fn strict_s3_registrar_tables_are_decoded_and_added_to_builtin() {
        let root = write_config(
            "[inline-r.substitutions]\n'custom()' = '\\code{custom}'\n\n[[s3.registrars]]\nfunction = 'register_s3_method'\narguments = ['class', 'generic', 'method']\n",
        );
        let loaded = load(root.path())
            .expect("configuration should load")
            .expect("config");
        assert!(
            loaded
                .registrars
                .signatures()
                .iter()
                .any(|signature| signature.callee() == "s3_register")
        );
        assert!(
            loaded
                .registrars
                .signatures()
                .iter()
                .any(|signature| signature.callee() == "register_s3_method")
        );
        assert_eq!(loaded.entries["custom()"], r#"\code{custom}"#);
    }

    #[test]
    fn duplicate_or_invalid_s3_registrar_tables_are_rejected() {
        let duplicate = write_config(
            "[[s3.registrars]]\nfunction = 's3_register'\narguments = ['generic', 'class']\n",
        );
        assert!(load(duplicate.path()).is_err());
        let invalid = write_config(
            "[[s3.registrars]]\nfunction = 'custom'\narguments = ['generic', 'generic']\n",
        );
        assert!(load(invalid.path()).is_err());
    }

    #[test]
    fn unreadable_configuration_is_an_io_error() {
        let root = tempdir().expect("temporary configuration directory");
        fs::create_dir(root.path().join(CONFIG_FILE)).expect("configuration path should exist");
        let error = load(root.path()).expect_err("directory should not be readable as config");
        assert!(matches!(error, ConfigError::Io { .. }));
    }
}
