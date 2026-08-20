use std::path::PathBuf;

use mini_roxygen_core::{
    DocumentOptions, ExternalInheritancePolicy, ExternalPolicySource, InheritanceOptions,
    InlineRSubstitutions, PackageInputs, document_package_with_options_and_providers,
};

use crate::args::DocArgs;
use crate::diagnostic::{render_diagnostics, render_output_errors, render_write_failure};
use crate::output::{WriteReport, apply_outputs, plan_outputs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Success,
    Diagnostics,
    Operational,
}

const fn status_for_diagnostics(has_errors: bool) -> Status {
    if has_errors {
        Status::Diagnostics
    } else {
        // Warnings and info diagnostics do not make documentation fail.
        Status::Success
    }
}

impl Status {
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Diagnostics => 1,
            Self::Operational => 2,
        }
    }
}

pub(crate) fn run(args: DocArgs) -> Status {
    let r_lib_paths = args.effective_r_lib_paths();
    let inputs = match PackageInputs::from_package_root(&args.package_path) {
        Ok(inputs) => inputs,
        Err(error) => {
            eprintln!("error: {error}");
            return Status::Operational;
        }
    };
    let (substitutions, registrars) = match crate::config::load(&args.package_path) {
        Ok(Some(config)) => {
            match InlineRSubstitutions::from_user_entries(config.entries, Some(config.origin)) {
                Ok(substitutions) => (substitutions, config.registrars),
                Err(diagnostics) => {
                    let rendered = render_diagnostics(&inputs.sources, diagnostics.iter());
                    if !rendered.is_empty() {
                        eprintln!("{rendered}");
                    }
                    return Status::Diagnostics;
                }
            }
        }
        Ok(None) => match InlineRSubstitutions::builtins() {
            Ok(substitutions) => (substitutions, Default::default()),
            Err(error) => {
                eprintln!("error: invalid built-in substitution: {error}");
                return Status::Operational;
            }
        },
        Err(error) => {
            eprintln!(
                "invalid mini-roxygen configuration {}: {error}",
                error.path().display()
            );
            return Status::Operational;
        }
    };
    let options = DocumentOptions {
        inline_r_substitutions: substitutions,
        s3_registrars: registrars,
    };
    let loaded = crate::installed::load_providers(&r_lib_paths, inputs.metadata.dependencies());
    for warning in &loaded.warnings {
        eprintln!("{}", crate::installed::render_warning(warning));
    }
    let inheritance_options = if r_lib_paths.is_empty() {
        InheritanceOptions {
            external: ExternalInheritancePolicy::Off,
            external_source: ExternalPolicySource::NoConfiguredLibrary,
        }
    } else {
        InheritanceOptions {
            external: ExternalInheritancePolicy::BestEffort,
            external_source: ExternalPolicySource::Explicit,
        }
    };
    let output = document_package_with_options_and_providers(
        &inputs,
        &options,
        &loaded.s3,
        &loaded.documentation,
        &inheritance_options,
    );
    let rendered = render_diagnostics(&inputs.sources, output.diagnostics());
    if !rendered.is_empty() {
        eprintln!("{rendered}");
    }
    write_gate(
        output.has_errors(),
        || match plan_outputs(&args.package_path, &output.rd, &output.namespace) {
            Ok(plan) => Ok(plan),
            Err(errors) => {
                eprintln!("{}", render_output_errors(&errors));
                Err(Status::Operational)
            }
        },
        |plan| match apply_outputs(plan) {
            Ok(report) => {
                report_written(&report);
                Ok(())
            }
            Err(failure) => {
                eprintln!("{}", render_write_failure(&failure));
                Err(Status::Operational)
            }
        },
    )
}

/// Names every file the run actually wrote, in output-path order.
///
/// Files left alone because they already held the generated content are not
/// named: silence means the tree already agreed with the source. roxygen2
/// reports the same way, and a run that says nothing at all would leave the
/// user unable to tell writing from doing nothing.
fn report_written(report: &WriteReport) {
    let mut written: Vec<&PathBuf> = report
        .created
        .iter()
        .chain(report.replaced.iter())
        .collect();
    written.sort_unstable();
    for path in written {
        eprintln!("Writing {}", path.display());
    }
}

fn write_gate<T>(
    has_errors: bool,
    plan: impl FnOnce() -> Result<T, Status>,
    apply: impl FnOnce(T) -> Result<(), Status>,
) -> Status {
    if has_errors {
        return status_for_diagnostics(true);
    }
    let plan = match plan() {
        Ok(plan) => plan,
        Err(status) => return status,
    };
    match apply(plan) {
        Ok(()) => Status::Success,
        Err(status) => status,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::cell::Cell;
    use tempfile::tempdir;

    use super::{Status, run, write_gate};
    use crate::args::DocArgs;

    #[test]
    fn status_mapping_matches_command_outcomes() {
        assert_eq!(super::status_for_diagnostics(false).exit_code(), 0);
        assert_eq!(super::status_for_diagnostics(true).exit_code(), 1);
        assert_eq!(Status::Operational.exit_code(), 2);
    }

    #[test]
    fn error_diagnostics_skip_planning_and_application() {
        let planned = Cell::new(false);
        let applied = Cell::new(false);
        let status = write_gate(
            true,
            || {
                planned.set(true);
                Ok::<(), Status>(())
            },
            |_| {
                applied.set(true);
                Ok::<(), Status>(())
            },
        );

        assert_eq!(status, Status::Diagnostics);
        assert!(!planned.get());
        assert!(!applied.get());
    }

    #[test]
    fn missing_description_is_operational_and_writes_nothing() {
        let root = tempdir().expect("temporary package root");
        let status = run(DocArgs {
            package_path: root.path().to_owned(),
            r_lib_paths: Vec::new(),
            r_lib_paths_list: None,
        });

        assert_eq!(status, Status::Operational);
        assert_eq!(
            fs::read_dir(root.path())
                .expect("temporary package root should be readable")
                .count(),
            0
        );
    }
}
