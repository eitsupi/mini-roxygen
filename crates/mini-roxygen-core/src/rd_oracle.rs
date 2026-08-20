//! Checks generated Rd against R's own parser during tests.
//!
//! `rd-writer` reparses what it serializes, which proves the AST survives a
//! round trip through its own reader. It does not prove that R accepts the
//! file. This module closes that gap by handing the text to
//! `tools::parse_Rd`.
//!
//! R is a development dependency only — the generator itself never runs R.
//! When R is unavailable the check reports that it was skipped instead of
//! passing quietly, and setting `MINI_ROXYGEN_REQUIRE_RD_ORACLE` turns a skip
//! into a failure so a machine that is supposed to have R cannot lose the
//! check by accident.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::Builder;

const REQUIRE_ENV: &str = "MINI_ROXYGEN_REQUIRE_RD_ORACLE";

/// The outcome of asking R to parse an Rd document.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OracleOutcome {
    /// R parsed the document with no errors and no warnings.
    Accepted,
    /// R reported at least one error or warning, one per entry.
    Rejected(Vec<String>),
    /// R was not available, and the check was not required.
    Skipped(String),
}

fn oracle_script() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/parse-rd.R")
        .canonicalize()
        .map_err(|error| format!("the Rd oracle script is unavailable: {error}"))
}

fn rd2ex_oracle_script() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/rd2ex-parse.R")
        .canonicalize()
        .map_err(|error| format!("the Rd2ex oracle script is unavailable: {error}"))
}

fn skip_or_require(reason: impl Into<String>, required: bool) -> OracleOutcome {
    let reason = reason.into();
    if required {
        panic!("{REQUIRE_ENV} is set but {reason}");
    }
    OracleOutcome::Skipped(reason)
}

fn strwrap_oracle_script() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/oracle/strwrap.R")
        .canonicalize()
        .map_err(|error| format!("the strwrap oracle script is unavailable: {error}"))
}

/// Compares Rust's generation-header wrapping with R's strwrap.
#[track_caller]
pub(crate) fn assert_r_strwrap_matches(cases: &[String], expected: &[Vec<String>]) {
    assert_eq!(cases.len(), expected.len());
    let input = cases.join("\n");
    let output = match run_strwrap_oracle(&input) {
        Ok(output) => output,
        Err(OracleOutcome::Skipped(reason)) => {
            eprintln!(
                "strwrap oracle skipped ({reason}); set {REQUIRE_ENV} to make this a failure"
            );
            return;
        }
        Err(OracleOutcome::Rejected(findings)) => {
            panic!("the strwrap oracle failed:\n{}", findings.join("\n"));
        }
        Err(OracleOutcome::Accepted) => unreachable!(),
    };

    let mut actual = Vec::new();
    let mut active = false;
    for line in output.lines() {
        if let Some(index) = line.strip_prefix("CASE ") {
            assert!(
                !active,
                "the strwrap oracle started a case before ending one"
            );
            let index = index
                .parse::<usize>()
                .expect("strwrap oracle case index should be numeric");
            assert_eq!(index, actual.len() + 1);
            actual.push(Vec::new());
            active = true;
        } else if let Some(line) = line.strip_prefix("LINE ") {
            assert!(active, "strwrap oracle produced a line without a case");
            actual
                .last_mut()
                .expect("strwrap oracle case should exist")
                .push(line.to_owned());
        } else if line == "END" {
            assert!(active, "strwrap oracle ended a case that was not open");
            active = false;
        } else if line == "STATUS ok" {
            continue;
        } else {
            panic!("the strwrap oracle produced unexpected output: {line}");
        }
    }
    assert!(!active, "the strwrap oracle ended inside a case");
    assert_eq!(actual, expected);
}

fn run_strwrap_oracle(input: &str) -> Result<String, OracleOutcome> {
    let script = match strwrap_oracle_script() {
        Ok(script) => script,
        Err(reason) => {
            return Err(skip_or_require(
                reason,
                std::env::var_os(REQUIRE_ENV).is_some(),
            ));
        }
    };
    let workspace = Builder::new()
        .prefix("mini-roxygen-strwrap-oracle-")
        .tempdir()
        .expect("the strwrap oracle must be able to create a temporary workspace");
    let path = workspace.path().join("cases.txt");
    if let Err(error) = fs::write(&path, input) {
        panic!(
            "the strwrap oracle must be able to write {}: {error}",
            path.display()
        );
    }
    let output = Command::new("Rscript")
        .arg("--vanilla")
        .arg(script)
        .arg(path)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            let reason = format!("Rscript could not be run: {error}");
            return Err(skip_or_require(
                reason,
                std::env::var_os(REQUIRE_ENV).is_some(),
            ));
        }
    };
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(OracleOutcome::Rejected(vec![format!(
            "Rscript exited unsuccessfully: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )]))
    }
}

/// Wraps an Rd body in the smallest topic that `parse_Rd` accepts as a file.
///
/// A converted Markdown value is a fragment, and `fragment = FALSE` needs the
/// surrounding topic, so tests that only care about one section can still ask
/// R about it.
pub(crate) fn minimal_topic(description: &str) -> String {
    format!(
        "\\name{{oracle}}\n\\alias{{oracle}}\n\\title{{Oracle}}\n\\description{{\n{description}\n}}\n"
    )
}

/// Asks R whether it accepts this Rd document.
pub(crate) fn parse_rd(document: &str) -> OracleOutcome {
    parse_rd_with_script(
        document,
        oracle_script(),
        std::env::var_os(REQUIRE_ENV).is_some(),
    )
}

/// Fails the calling test unless R can extract and parse the examples as R.
#[track_caller]
pub(crate) fn assert_r_examples_parse(document: &str) {
    let outcome = parse_rd_with_script(
        document,
        rd2ex_oracle_script(),
        std::env::var_os(REQUIRE_ENV).is_some(),
    );
    match outcome {
        OracleOutcome::Accepted => {}
        OracleOutcome::Rejected(findings) => {
            panic!(
                "R rejected the Rd2ex extraction:\n{document}\nfindings:\n{}",
                findings.join("\n")
            );
        }
        OracleOutcome::Skipped(reason) => {
            eprintln!("Rd2ex oracle skipped ({reason}); set {REQUIRE_ENV} to make this a failure");
        }
    }
}

fn parse_rd_with_script(
    document: &str,
    script: Result<PathBuf, String>,
    required: bool,
) -> OracleOutcome {
    let script = match script {
        Ok(script) => script,
        Err(reason) => return skip_or_require(reason, required),
    };
    let workspace = Builder::new()
        .prefix("mini-roxygen-oracle-")
        .tempdir()
        .expect("the Rd oracle must be able to create a temporary workspace");
    let path = workspace.path().join("document.Rd");
    if let Err(error) = fs::write(&path, document) {
        panic!(
            "the Rd oracle must be able to write {}: {error}",
            path.display()
        );
    }

    let output = Command::new("Rscript")
        .arg("--vanilla")
        .arg(&script)
        .arg(&path)
        .output();

    let output = match output {
        Ok(output) => output,
        Err(error) => {
            let reason = format!("Rscript could not be run: {error}");
            return skip_or_require(reason, required);
        }
    };

    interpret_output(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

fn interpret_output(success: bool, stdout: &str, stderr: &str) -> OracleOutcome {
    let findings: Vec<String> = stdout
        .lines()
        .filter(|line| line.starts_with("ERROR ") || line.starts_with("WARNING "))
        .map(str::to_owned)
        .collect();

    if success && stdout.lines().any(|line| line == "STATUS ok") {
        return OracleOutcome::Accepted;
    }
    if findings.is_empty() {
        panic!("the Rd oracle produced no verdict.\nstdout:\n{stdout}\nstderr:\n{stderr}");
    }
    OracleOutcome::Rejected(findings)
}

/// Fails the calling test unless R accepts this Rd document.
#[track_caller]
pub(crate) fn assert_r_accepts(document: &str) {
    match parse_rd(document) {
        OracleOutcome::Accepted => {}
        OracleOutcome::Rejected(findings) => {
            panic!(
                "R rejected this Rd:\n{document}\nfindings:\n{}",
                findings.join("\n")
            );
        }
        OracleOutcome::Skipped(reason) => {
            eprintln!("Rd oracle skipped ({reason}); set {REQUIRE_ENV} to make this a failure");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OracleOutcome, assert_r_accepts, interpret_output, minimal_topic, parse_rd,
        parse_rd_with_script,
    };

    #[test]
    fn r_accepts_a_minimal_topic() {
        assert_r_accepts(&minimal_topic(
            "Ordinary prose with 100\\% and \\emph{emphasis}.",
        ));
    }

    #[test]
    fn the_oracle_reports_what_r_rejects() {
        // Without this the oracle could accept everything and no test would
        // notice, which is the failure mode a parser check exists to avoid.
        let outcome =
            parse_rd("\\name{oracle}\n\\title{Oracle}\n\\description{unbalanced \\emph{x}\n");
        match outcome {
            OracleOutcome::Rejected(findings) => {
                assert!(
                    findings
                        .iter()
                        .any(|finding| finding.contains("END_OF_INPUT")),
                    "expected an unexpected-end finding, got {findings:?}"
                );
            }
            OracleOutcome::Skipped(_) => {}
            OracleOutcome::Accepted => panic!("the oracle accepted unbalanced Rd"),
        }
    }

    #[test]
    fn an_unescaped_percent_swallows_the_rest_of_its_line() {
        // `%` starts an Rd comment, so a converter that emits one literally
        // instead of leaving the escaping to the writer silently drops the
        // rest of the line — here the brace that closes `\emph`.
        let outcome = parse_rd(&minimal_topic("\\emph{text % }"));
        assert_ne!(outcome, OracleOutcome::Accepted);
    }

    #[test]
    fn an_unavailable_script_is_skipped_when_oracle_is_not_required() {
        let outcome = parse_rd_with_script(
            "document",
            Err("the test oracle script is unavailable".to_owned()),
            false,
        );
        assert_eq!(
            outcome,
            OracleOutcome::Skipped("the test oracle script is unavailable".to_owned())
        );
    }

    #[test]
    fn an_unsuccessful_process_cannot_be_accepted_by_its_status_line() {
        let outcome = interpret_output(false, "ERROR parser failed\nSTATUS ok\n", "failure");
        assert_eq!(
            outcome,
            OracleOutcome::Rejected(vec!["ERROR parser failed".to_owned()])
        );
    }

    #[test]
    fn concurrent_oracle_conversions_use_independent_workspaces() {
        std::thread::scope(|scope| {
            let first = scope.spawn(|| parse_rd(&minimal_topic("first")));
            let second = scope.spawn(|| parse_rd(&minimal_topic("second")));
            let first = first.join().expect("first oracle conversion did not panic");
            let second = second
                .join()
                .expect("second oracle conversion did not panic");
            assert!(matches!(
                first,
                OracleOutcome::Accepted | OracleOutcome::Skipped(_)
            ));
            assert!(matches!(
                second,
                OracleOutcome::Accepted | OracleOutcome::Skipped(_)
            ));
        });
    }
}
