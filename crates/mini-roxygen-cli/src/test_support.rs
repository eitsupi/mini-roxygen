use std::env;
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn installed_r_libraries() -> Vec<PathBuf> {
    let candidates = if let Some(paths) = env::var_os("MINI_ROXYGEN_TEST_R_LIB_PATHS") {
        env::split_paths(&paths).collect::<Vec<_>>()
    } else if let Some(home) = env::var_os("R_HOME") {
        vec![PathBuf::from(home).join("library")]
    } else {
        Command::new("R")
            .arg("RHOME")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| parse_r_home_output(&String::from_utf8_lossy(&output.stdout)))
            .map(|home| vec![home.join("library")])
            .unwrap_or_default()
    };

    let mut libraries = Vec::new();
    for path in candidates {
        if path.is_dir() && !libraries.contains(&path) {
            libraries.push(path);
        }
    }
    libraries
}

pub(crate) fn installed_docs_required() -> bool {
    env::var("MINI_ROXYGEN_REQUIRE_INSTALLED_DOCS")
        .ok()
        .is_some_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no"
            )
        })
}

pub(crate) fn require_installed_documentation(libraries: &[PathBuf], package: &str) {
    if !installed_docs_required() {
        return;
    }

    assert!(
        !libraries.is_empty(),
        "MINI_ROXYGEN_REQUIRE_INSTALLED_DOCS is set but no R library is available"
    );
    assert!(
        libraries
            .iter()
            .any(|library| library.join(package).join("Meta/package.rds").is_file()),
        "MINI_ROXYGEN_REQUIRE_INSTALLED_DOCS is set but the installed {package} package is unavailable"
    );
}

pub(crate) fn parse_r_home_output(output: &str) -> Option<PathBuf> {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("WARNING:"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse_r_home_output;

    #[test]
    fn parses_r_home_output_around_warnings() {
        assert_eq!(
            parse_r_home_output("WARNING: startup notice\nfirst-r-home\n"),
            Some(PathBuf::from("first-r-home"))
        );
        assert_eq!(
            parse_r_home_output("first-r-home\nWARNING: trailing notice\nsecond-r-home\n"),
            Some(PathBuf::from("second-r-home"))
        );
        assert_eq!(parse_r_home_output("WARNING: no usable path\n"), None);
    }
}
