#![allow(dead_code)]
// The CLI command in a later task will consume this module; remove this allow then.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use mini_roxygen_core::{
    DiagnosticCode, FileClassification, NamespaceBuildOutput, RdBuildOutput, classify,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(not(unix))]
use tempfile::NamedTempFile;

const NAMESPACE_PATH: &str = "NAMESPACE";
const MAN_DIRECTORY: &str = "man";

/// The filesystem operation that failed while inspecting or writing output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputOperation {
    Read,
    CreateDirectory,
    CreateTemp,
    WriteTemp,
    SetPermissions,
    Persist,
}

impl fmt::Display for OutputOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self {
            Self::Read => "read",
            Self::CreateDirectory => "create directory",
            Self::CreateTemp => "create temporary file",
            Self::WriteTemp => "write temporary file",
            Self::SetPermissions => "set temporary file permissions",
            Self::Persist => "persist temporary file",
        };
        formatter.write_str(operation)
    }
}

/// An error that has no source-file span and therefore does not belong in a
/// core diagnostic.
#[derive(Debug)]
pub(crate) enum OutputError {
    UnmanagedOutputOverwrite {
        path: PathBuf,
    },
    PlanStale {
        path: PathBuf,
        planned: OutputAction,
        observed: OutputAction,
    },
    InvalidOutputPath {
        path: PathBuf,
        reason: &'static str,
    },
    UnsafeOutputPath {
        path: PathBuf,
        reason: &'static str,
    },
    Io {
        operation: OutputOperation,
        path: PathBuf,
        source: io::Error,
    },
}

impl OutputError {
    /// Returns the stable code used for errors that are machine-actionable.
    pub(crate) const fn stable_code(&self) -> Option<&'static str> {
        match self {
            Self::UnmanagedOutputOverwrite { .. } => {
                Some(DiagnosticCode::UnmanagedOutputOverwrite.as_str())
            }
            Self::PlanStale { .. }
            | Self::InvalidOutputPath { .. }
            | Self::UnsafeOutputPath { .. }
            | Self::Io { .. } => None,
        }
    }
}

impl fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmanagedOutputOverwrite { path } => write!(
                formatter,
                "{}: refusing to overwrite hand-written output {}",
                DiagnosticCode::UnmanagedOutputOverwrite.as_str(),
                path.display()
            ),
            Self::PlanStale {
                path,
                planned,
                observed,
            } => write!(
                formatter,
                "stale output plan for {}: planned {planned:?}, observed {observed:?}",
                path.display()
            ),
            Self::InvalidOutputPath { path, reason } => {
                write!(
                    formatter,
                    "invalid output path {}: {reason}",
                    path.display()
                )
            }
            Self::UnsafeOutputPath { path, reason } => {
                write!(formatter, "unsafe output path {}: {reason}", path.display())
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for OutputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::UnmanagedOutputOverwrite { .. }
            | Self::PlanStale { .. }
            | Self::InvalidOutputPath { .. }
            | Self::UnsafeOutputPath { .. } => None,
        }
    }
}

/// All errors found while inspecting the output tree.
#[derive(Debug)]
pub(crate) struct OutputErrors {
    pub(crate) errors: Vec<OutputError>,
}

impl fmt::Display for OutputErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} output errors", self.errors.len())
    }
}

impl Error for OutputErrors {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

/// The kind of generated output represented by one plan entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputKind {
    Rd,
    Namespace,
}

/// The operation required for one output path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputAction {
    Create,
    Replace,
    Unchanged,
}

/// One validated output that borrows its generated content.
#[derive(Debug)]
pub(crate) struct PlannedOutput<'a> {
    pub(crate) relative_path: PathBuf,
    pub(crate) content: &'a str,
    pub(crate) kind: OutputKind,
    pub(crate) action: OutputAction,
}

/// A complete preflight result, ready for application.
#[derive(Debug)]
pub(crate) struct OutputPlan<'a> {
    root: PathBuf,
    entries: Vec<PlannedOutput<'a>>,
}

impl<'a> OutputPlan<'a> {
    /// Returns whether at least one output would be written.
    pub(crate) fn has_changes(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.action != OutputAction::Unchanged)
    }

    /// Iterates over entries in package-relative path order.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &PlannedOutput<'a>> {
        self.entries.iter()
    }
}

/// The paths affected by a successful application.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WriteReport {
    pub(crate) created: Vec<PathBuf>,
    pub(crate) replaced: Vec<PathBuf>,
    pub(crate) unchanged: Vec<PathBuf>,
}

/// A write failure together with the progress made before it occurred.
#[derive(Debug)]
pub(crate) struct WriteFailure {
    pub(crate) error: OutputError,
    pub(crate) completed: WriteReport,
    pub(crate) not_attempted: Vec<PathBuf>,
}

struct Candidate<'a> {
    relative_path: PathBuf,
    content: &'a str,
    kind: OutputKind,
}

enum ManStatus {
    Missing,
    Existing,
}

/// Inspects every generated destination and returns a deterministic write plan.
pub(crate) fn plan_outputs<'a>(
    package_root: &Path,
    rd: &'a RdBuildOutput,
    namespace: &'a NamespaceBuildOutput,
) -> Result<OutputPlan<'a>, OutputErrors> {
    let root = match fs::canonicalize(package_root) {
        Ok(root) => root,
        Err(source) => {
            return Err(OutputErrors {
                errors: vec![OutputError::Io {
                    operation: OutputOperation::Read,
                    path: package_root.to_owned(),
                    source,
                }],
            });
        }
    };

    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(OutputErrors {
                errors: vec![OutputError::UnsafeOutputPath {
                    path: root,
                    reason: "package root is not a directory",
                }],
            });
        }
        Err(source) => {
            return Err(OutputErrors {
                errors: vec![OutputError::Io {
                    operation: OutputOperation::Read,
                    path: root,
                    source,
                }],
            });
        }
    }

    let mut candidates = rd
        .files
        .values()
        .map(|file| Candidate {
            relative_path: file.relative_path.clone(),
            content: &file.content,
            kind: OutputKind::Rd,
        })
        .chain(std::iter::once(Candidate {
            relative_path: PathBuf::from(NAMESPACE_PATH),
            content: &namespace.content,
            kind: OutputKind::Namespace,
        }))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut errors = Vec::new();
    let mut entries = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        if let Err(error) = validate_output_path(&candidate.relative_path, candidate.kind) {
            errors.push(error);
            continue;
        }

        let destination = root.join(&candidate.relative_path);
        if candidate.kind == OutputKind::Rd {
            match inspect_man_directory(&root) {
                Ok(ManStatus::Missing) => {
                    entries.push(PlannedOutput {
                        relative_path: candidate.relative_path,
                        content: candidate.content,
                        kind: candidate.kind,
                        action: OutputAction::Create,
                    });
                    continue;
                }
                Ok(ManStatus::Existing) => {}
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            }
        }

        match inspect_destination(&destination, candidate.content) {
            Ok(action) => entries.push(PlannedOutput {
                relative_path: candidate.relative_path,
                content: candidate.content,
                kind: candidate.kind,
                action,
            }),
            Err(error) => errors.push(error),
        }
    }

    if !errors.is_empty() {
        return Err(OutputErrors { errors });
    }

    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(OutputPlan { root, entries })
}

/// Applies a plan in relative-path order, atomically replacing one file at a time.
#[allow(clippy::result_large_err)]
pub(crate) fn apply_outputs(plan: OutputPlan<'_>) -> Result<WriteReport, WriteFailure> {
    let mut completed = WriteReport::default();

    for (index, entry) in plan.entries.iter().enumerate() {
        let result = revalidate_entry(&plan.root, entry).and_then(|()| match entry.action {
            OutputAction::Unchanged => Ok(()),
            OutputAction::Create | OutputAction::Replace => write_entry(&plan.root, entry),
        });

        if let Err(error) = result {
            return Err(WriteFailure {
                error,
                completed,
                not_attempted: plan.entries[index + 1..]
                    .iter()
                    .map(|entry| entry.relative_path.clone())
                    .collect(),
            });
        }

        match entry.action {
            OutputAction::Create => completed.created.push(entry.relative_path.clone()),
            OutputAction::Replace => completed.replaced.push(entry.relative_path.clone()),
            OutputAction::Unchanged => completed.unchanged.push(entry.relative_path.clone()),
        }
    }

    Ok(completed)
}

fn validate_output_path(path: &Path, kind: OutputKind) -> Result<(), OutputError> {
    if kind == OutputKind::Namespace {
        if path == Path::new(NAMESPACE_PATH) {
            return Ok(());
        }
        return Err(invalid_path(
            path,
            "NAMESPACE must be package-root relative",
        ));
    }

    if path.is_absolute() {
        return Err(invalid_path(path, "absolute paths are not permitted"));
    }

    let mut components = path.components();
    let first = components.next();
    let second = components.next();
    if components.next().is_some() {
        return Err(invalid_path(
            path,
            "an Rd path must have exactly two components",
        ));
    }

    let Some(Component::Normal(parent)) = first else {
        return Err(invalid_path(path, "the first component must be man"));
    };
    let Some(Component::Normal(filename)) = second else {
        return Err(invalid_path(
            path,
            "the second component must be an Rd filename",
        ));
    };
    if parent != OsStr::new(MAN_DIRECTORY) {
        return Err(invalid_path(path, "the first component must be man"));
    }

    let Some(filename) = filename.to_str() else {
        return Err(invalid_path(path, "the filename must be ASCII"));
    };
    let Some(stem) = filename.strip_suffix(".Rd") else {
        return Err(invalid_path(
            path,
            "the filename extension must be exactly .Rd",
        ));
    };
    if stem.is_empty() || filename == "." || filename == ".." {
        return Err(invalid_path(
            path,
            "the Rd filename must not be empty or dot-like",
        ));
    }
    if !filename
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(invalid_path(
            path,
            "the filename contains a character outside [A-Za-z0-9_.-]",
        ));
    }
    if is_windows_reserved_device_name(filename) {
        return Err(invalid_path(
            path,
            "the filename uses a Windows device name",
        ));
    }

    Ok(())
}

/// Reports whether a filename names a Windows character device.
///
/// Windows resolves a device name from the segment before the first period, so
/// `CON.default.Rd` names the console just as `CON.Rd` does. Testing the
/// extension-stripped stem would miss every dotted R topic name, and dots are
/// ordinary in R identifiers.
fn is_windows_reserved_device_name(filename: &str) -> bool {
    let leading = filename.split('.').next().unwrap_or(filename);
    let uppercase = leading.to_ascii_uppercase();
    matches!(
        uppercase.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn invalid_path(path: &Path, reason: &'static str) -> OutputError {
    OutputError::InvalidOutputPath {
        path: path.to_owned(),
        reason,
    }
}

fn inspect_man_directory(root: &Path) -> Result<ManStatus, OutputError> {
    let path = root.join(MAN_DIRECTORY);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => Err(OutputError::UnsafeOutputPath {
            path,
            reason: "man is a symlink or reparse point",
        }),
        Ok(metadata) if !metadata.file_type().is_dir() => Err(OutputError::UnsafeOutputPath {
            path,
            reason: "man is not a directory",
        }),
        Ok(_) => Ok(ManStatus::Existing),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(ManStatus::Missing),
        Err(source) => Err(OutputError::Io {
            operation: OutputOperation::Read,
            path,
            source,
        }),
    }
}

fn ensure_man_directory(root: &Path) -> Result<(), OutputError> {
    let path = root.join(MAN_DIRECTORY);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&path).map_err(|source| OutputError::Io {
                operation: OutputOperation::CreateDirectory,
                path: path.clone(),
                source,
            })?;
        }
        Err(source) => {
            return Err(OutputError::Io {
                operation: OutputOperation::Read,
                path,
                source,
            });
        }
    }

    match fs::symlink_metadata(&path) {
        Ok(metadata) if is_link_or_reparse(&metadata) => Err(OutputError::UnsafeOutputPath {
            path,
            reason: "man is a symlink or reparse point",
        }),
        Ok(metadata) if !metadata.file_type().is_dir() => Err(OutputError::UnsafeOutputPath {
            path,
            reason: "man is not a directory",
        }),
        Ok(_) => Ok(()),
        Err(source) => Err(OutputError::Io {
            operation: OutputOperation::Read,
            path,
            source,
        }),
    }
}

fn inspect_destination(path: &Path, new_content: &str) -> Result<OutputAction, OutputError> {
    Ok(inspect_destination_details(path, new_content)?.action)
}

struct DestinationInspection {
    action: OutputAction,
    metadata: Option<fs::Metadata>,
}

fn inspect_destination_details(
    path: &Path,
    new_content: &str,
) -> Result<DestinationInspection, OutputError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(DestinationInspection {
                action: OutputAction::Create,
                metadata: None,
            });
        }
        Err(source) => {
            return Err(OutputError::Io {
                operation: OutputOperation::Read,
                path: path.to_owned(),
                source,
            });
        }
    };
    validate_destination_metadata(path, &metadata)?;

    let bytes = fs::read(path).map_err(|source| OutputError::Io {
        operation: OutputOperation::Read,
        path: path.to_owned(),
        source,
    })?;
    let existing = std::str::from_utf8(&bytes).map_err(|source| OutputError::Io {
        operation: OutputOperation::Read,
        path: path.to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })?;
    if matches!(classify(existing), FileClassification::HandWritten) {
        return Err(OutputError::UnmanagedOutputOverwrite {
            path: path.to_owned(),
        });
    }

    #[cfg(windows)]
    if metadata.permissions().readonly() {
        return Err(OutputError::UnsafeOutputPath {
            path: path.to_owned(),
            reason: "the destination is read-only",
        });
    }

    if bytes == new_content.as_bytes() {
        Ok(DestinationInspection {
            action: OutputAction::Unchanged,
            metadata: Some(metadata),
        })
    } else {
        Ok(DestinationInspection {
            action: OutputAction::Replace,
            metadata: Some(metadata),
        })
    }
}

fn revalidate_entry(root: &Path, entry: &PlannedOutput<'_>) -> Result<(), OutputError> {
    validate_output_path(&entry.relative_path, entry.kind)?;
    if entry.kind == OutputKind::Rd {
        inspect_man_directory(root)?;
    }

    let destination = root.join(&entry.relative_path);
    let observed = inspect_destination(&destination, entry.content)?;
    if observed != entry.action {
        return Err(OutputError::PlanStale {
            path: destination,
            planned: entry.action,
            observed,
        });
    }
    Ok(())
}

fn validate_destination_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), OutputError> {
    if is_link_or_reparse(metadata) {
        return Err(OutputError::UnsafeOutputPath {
            path: path.to_owned(),
            reason: "the destination is a symlink or reparse point",
        });
    }
    if !metadata.file_type().is_file() {
        return Err(OutputError::UnsafeOutputPath {
            path: path.to_owned(),
            reason: "the destination is not a regular file",
        });
    }
    Ok(())
}

fn write_entry(root: &Path, entry: &PlannedOutput<'_>) -> Result<(), OutputError> {
    let parent = match entry.kind {
        OutputKind::Rd => {
            ensure_man_directory(root)?;
            root.join(MAN_DIRECTORY)
        }
        OutputKind::Namespace => root.to_owned(),
    };
    let destination = root.join(&entry.relative_path);

    // A path-based API cannot exclude a TOCTOU swap between these checks and
    // the rename. Immediate re-checks are sufficient for this local codegen tool.
    check_destination_before_write(&destination, entry.content, entry.action)?;

    // Creation uses 0666 so the kernel applies the umask; replacement copies
    // the existing rwx bits explicitly because applying 0666 would mask them.
    #[cfg(unix)]
    let mut temporary = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o666))
        .tempfile_in(&parent)
        .map_err(|source| OutputError::Io {
            operation: OutputOperation::CreateTemp,
            path: destination.clone(),
            source,
        })?;
    #[cfg(not(unix))]
    let mut temporary = NamedTempFile::new_in(&parent).map_err(|source| OutputError::Io {
        operation: OutputOperation::CreateTemp,
        path: destination.clone(),
        source,
    })?;
    temporary
        .write_all(entry.content.as_bytes())
        .map_err(|source| OutputError::Io {
            operation: OutputOperation::WriteTemp,
            path: destination.clone(),
            source,
        })?;
    temporary.flush().map_err(|source| OutputError::Io {
        operation: OutputOperation::WriteTemp,
        path: destination.clone(),
        source,
    })?;

    #[cfg(unix)]
    let destination_metadata =
        check_destination_before_write(&destination, entry.content, entry.action)?;
    #[cfg(not(unix))]
    check_destination_before_write(&destination, entry.content, entry.action)?;

    #[cfg(unix)]
    if entry.action == OutputAction::Replace
        && let Some(metadata) = destination_metadata
    {
        let mode = metadata.permissions().mode() & 0o777;
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(mode)).map_err(
            |source| OutputError::Io {
                operation: OutputOperation::SetPermissions,
                path: destination.clone(),
                source,
            },
        )?;
    }

    // Deliberately omit fsync/sync_all: atomic visibility is required, but
    // database-grade power-loss durability is not, and these outputs are
    // regenerable from source.
    temporary
        .persist(&destination)
        .map_err(|error| OutputError::Io {
            operation: OutputOperation::Persist,
            path: destination,
            source: error.error,
        })?;
    Ok(())
}

/// Re-inspects a destination just before it is written.
///
/// The observed action is compared against the planned one, not merely
/// discarded: a destination that gained or lost a file between planning and
/// this moment would otherwise be created over or silently recreated. Only the
/// race after this check remains, and that one is the documented residual.
fn check_destination_before_write(
    path: &Path,
    content: &str,
    planned: OutputAction,
) -> Result<Option<fs::Metadata>, OutputError> {
    let inspection = inspect_destination_details(path, content)?;
    if inspection.action != planned {
        return Err(OutputError::PlanStale {
            path: path.to_owned(),
            planned,
            observed: inspection.action,
        });
    }
    Ok(inspection.metadata)
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use mini_roxygen_core::{
        Diagnostics, GeneratedRd, NamespaceBuildOutput, RdBuildOutput, TopicKey,
    };
    use rd_ast::RdDocument;
    use tempfile::tempdir;

    use super::{
        OutputAction, OutputError, OutputKind, OutputOperation, apply_outputs, plan_outputs,
    };

    /// Builds Rd outputs whose documents are empty.
    ///
    /// The writer persists `content` and never reads `document`, so an empty
    /// tree states that premise directly. Generating a real one would tie
    /// these tests to the whole parsing and inheritance pipeline without
    /// exercising any of it.
    fn rd_output(entries: impl IntoIterator<Item = (&'static str, &'static str)>) -> RdBuildOutput {
        let files = entries
            .into_iter()
            .enumerate()
            .map(|(index, (path, content))| {
                (
                    TopicKey(format!("topic-{index}")),
                    GeneratedRd {
                        relative_path: PathBuf::from(path),
                        document: RdDocument::new(Vec::new()),
                        content: content.to_owned(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        RdBuildOutput {
            files,
            diagnostics: Diagnostics::new(),
        }
    }

    fn namespace(content: &'static str) -> NamespaceBuildOutput {
        NamespaceBuildOutput {
            content: content.to_owned(),
            diagnostics: Diagnostics::new(),
        }
    }

    fn plan<'a>(
        root: &Path,
        rd: &'a RdBuildOutput,
        namespace: &'a NamespaceBuildOutput,
    ) -> super::OutputPlan<'a> {
        plan_outputs(root, rd, namespace).expect("output preflight succeeds")
    }

    fn has_error<T>(
        result: Result<T, super::OutputErrors>,
        predicate: impl Fn(&OutputError) -> bool,
    ) {
        let errors = match result {
            Ok(_) => panic!("preflight should fail"),
            Err(errors) => errors,
        };
        assert!(
            errors.errors.iter().any(predicate),
            "errors: {:?}",
            errors.errors
        );
    }

    #[test]
    fn creates_and_replaces_both_output_kinds_atomically() {
        let root = tempdir().expect("temporary package root");
        let rd_content =
            "% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nnew Rd\n";
        let namespace_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nexport(foo)\n";
        let rd = rd_output([("man/foo.Rd", rd_content)]);
        let namespace_output = namespace(namespace_content);
        let report =
            apply_outputs(plan(root.path(), &rd, &namespace_output)).expect("write succeeds");
        assert_eq!(
            report.created,
            vec![PathBuf::from("NAMESPACE"), PathBuf::from("man/foo.Rd")]
        );
        assert_eq!(
            fs::read(root.path().join("NAMESPACE")).unwrap(),
            namespace_content.as_bytes()
        );
        assert_eq!(
            fs::read(root.path().join("man/foo.Rd")).unwrap(),
            rd_content.as_bytes()
        );

        let replacement = "# Generated by roxygen2: do not edit by hand\nreplacement\n";
        fs::write(root.path().join("NAMESPACE"), replacement).unwrap();
        fs::write(
            root.path().join("man/foo.Rd"),
            "% Generated by cpp11: do not edit by hand\nold replacement\n",
        )
        .unwrap();
        let rd2 = rd_output([(
            "man/foo.Rd",
            "% Generated by cpp11: do not edit by hand\nreplacement\n",
        )]);
        let namespace2 = namespace("# Generated by other: do not edit by hand\nreplacement\n");
        let report =
            apply_outputs(plan(root.path(), &rd2, &namespace2)).expect("replacement succeeds");
        assert_eq!(
            report.replaced,
            vec![PathBuf::from("NAMESPACE"), PathBuf::from("man/foo.Rd")]
        );
        assert_eq!(
            fs::read_to_string(root.path().join("NAMESPACE")).unwrap(),
            namespace2.content
        );
        assert_eq!(
            fs::read_to_string(root.path().join("man/foo.Rd")).unwrap(),
            rd2.files.values().next().unwrap().content
        );
    }

    #[cfg(unix)]
    #[test]
    fn created_files_use_the_normal_umask_applied_mode() {
        let root = tempdir().unwrap();
        let namespace_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\n";
        let rd = rd_output([]);
        let namespace_output = namespace(namespace_content);
        let output_plan = plan(root.path(), &rd, &namespace_output);

        apply_outputs(output_plan).expect("write succeeds");

        // Compare against a file written the ordinary way rather than a fixed
        // 0644: the writer asks for 0666 and lets the kernel apply the umask,
        // so the expected mode is whatever this process's umask produces.
        let control = root.path().join("control");
        fs::write(&control, "").unwrap();
        let expected = fs::metadata(&control).unwrap().permissions().mode() & 0o777;
        let mode = fs::metadata(root.path().join("NAMESPACE"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, expected);
    }

    #[cfg(unix)]
    #[test]
    fn replaced_files_preserve_existing_rwx_mode() {
        let root = tempdir().unwrap();
        let path = root.path().join("NAMESPACE");
        let old = "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nold\n";
        let new = "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nnew\n";
        fs::write(&path, old).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let rd = rd_output([]);
        let namespace_output = namespace(new);
        let output_plan = plan(root.path(), &rd, &namespace_output);
        apply_outputs(output_plan).expect("replacement succeeds");

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn refuses_hand_written_rd_and_namespace_without_modifying_them() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("man")).unwrap();
        fs::write(root.path().join("man/foo.Rd"), "hand-written Rd\n").unwrap();
        fs::write(root.path().join("NAMESPACE"), "hand-written namespace\n").unwrap();
        let before_rd = fs::read(root.path().join("man/foo.Rd")).unwrap();
        let before_namespace = fs::read(root.path().join("NAMESPACE")).unwrap();
        let rd = rd_output([(
            "man/foo.Rd",
            "% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nnew\n",
        )]);
        let namespace_output = namespace(
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nnew\n",
        );
        let result = plan_outputs(root.path(), &rd, &namespace_output);
        let errors = result.expect_err("hand-written files must be refused");
        assert_eq!(errors.errors.len(), 2);
        assert!(
            errors
                .errors
                .iter()
                .all(|error| matches!(error, OutputError::UnmanagedOutputOverwrite { .. }))
        );
        assert_eq!(
            errors.errors[0].stable_code(),
            Some(mini_roxygen_core::DiagnosticCode::UnmanagedOutputOverwrite.as_str(),)
        );
        assert_eq!(fs::read(root.path().join("man/foo.Rd")).unwrap(), before_rd);
        assert_eq!(
            fs::read(root.path().join("NAMESPACE")).unwrap(),
            before_namespace
        );
    }

    #[test]
    fn identical_content_is_unchanged_and_keeps_mtime() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("man")).unwrap();
        let rd_content =
            "% Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nunchanged\n";
        fs::write(root.path().join("man/foo.Rd"), rd_content).unwrap();
        let before = fs::metadata(root.path().join("man/foo.Rd"))
            .unwrap()
            .modified()
            .unwrap();
        thread::sleep(Duration::from_millis(20));
        let rd = rd_output([("man/foo.Rd", rd_content)]);
        let namespace =
            namespace("# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\n");
        let output_plan = plan(root.path(), &rd, &namespace);
        assert_eq!(
            output_plan
                .entries()
                .find(|entry| entry.relative_path == Path::new("man/foo.Rd"))
                .unwrap()
                .action,
            OutputAction::Unchanged
        );
        let report = apply_outputs(output_plan).expect("unchanged output succeeds");
        assert_eq!(report.unchanged, vec![PathBuf::from("man/foo.Rd")]);
        let after = fs::metadata(root.path().join("man/foo.Rd"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn stale_unchanged_output_modified_after_planning_is_not_written() {
        let root = tempdir().unwrap();
        let path = root.path().join("NAMESPACE");
        let planned_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nplanned\n";
        let modified_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nmodified\n";
        fs::write(&path, planned_content).unwrap();
        let rd = rd_output([]);
        let namespace_output = namespace(planned_content);
        let output_plan = plan(root.path(), &rd, &namespace_output);
        fs::write(&path, modified_content).unwrap();

        let failure = apply_outputs(output_plan).expect_err("stale unchanged output must fail");
        assert!(matches!(
            failure.error,
            OutputError::PlanStale {
                planned: OutputAction::Unchanged,
                observed: OutputAction::Replace,
                ..
            }
        ));
        assert!(failure.completed.created.is_empty());
        assert!(failure.completed.replaced.is_empty());
        assert!(failure.completed.unchanged.is_empty());
        assert_eq!(fs::read_to_string(path).unwrap(), modified_content);
    }

    #[test]
    fn stale_unchanged_output_deleted_after_planning_is_not_written() {
        let root = tempdir().unwrap();
        let path = root.path().join("NAMESPACE");
        let planned_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nplanned\n";
        fs::write(&path, planned_content).unwrap();
        let rd = rd_output([]);
        let namespace_output = namespace(planned_content);
        let output_plan = plan(root.path(), &rd, &namespace_output);
        fs::remove_file(&path).unwrap();

        let failure = apply_outputs(output_plan).expect_err("deleted unchanged output must fail");
        assert!(matches!(
            failure.error,
            OutputError::PlanStale {
                planned: OutputAction::Unchanged,
                observed: OutputAction::Create,
                ..
            }
        ));
        assert!(failure.completed.created.is_empty());
        assert!(failure.completed.replaced.is_empty());
        assert!(failure.completed.unchanged.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn stale_unchanged_output_that_becomes_hand_written_is_refused() {
        let root = tempdir().unwrap();
        let path = root.path().join("NAMESPACE");
        let planned_content =
            "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nplanned\n";
        let hand_written_content = "hand-written\n";
        fs::write(&path, planned_content).unwrap();
        let rd = rd_output([]);
        let namespace_output = namespace(planned_content);
        let output_plan = plan(root.path(), &rd, &namespace_output);
        fs::write(&path, hand_written_content).unwrap();

        let failure = apply_outputs(output_plan).expect_err("hand-written output must fail");
        assert!(matches!(
            failure.error,
            OutputError::UnmanagedOutputOverwrite { .. }
        ));
        assert!(failure.completed.created.is_empty());
        assert!(failure.completed.replaced.is_empty());
        assert!(failure.completed.unchanged.is_empty());
        assert_eq!(fs::read_to_string(path).unwrap(), hand_written_content);
    }

    #[test]
    fn stale_replace_output_that_vanished_is_not_created() {
        let root = tempdir().unwrap();
        let path = root.path().join("NAMESPACE");
        let old = "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nold\n";
        let new = "# Generated by mini-roxygen (roxygen2 compatible): do not edit by hand\nnew\n";
        fs::write(&path, old).unwrap();
        let rd = rd_output([]);
        let namespace_output = namespace(new);
        let output_plan = plan(root.path(), &rd, &namespace_output);
        fs::remove_file(&path).unwrap();

        let failure = apply_outputs(output_plan).expect_err("stale replacement must fail");
        assert!(matches!(
            failure.error,
            OutputError::PlanStale {
                planned: OutputAction::Replace,
                observed: OutputAction::Create,
                ..
            }
        ));
        assert!(failure.completed.created.is_empty());
        assert!(failure.completed.replaced.is_empty());
        assert!(failure.completed.unchanged.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn preflight_collects_multiple_errors_before_writing() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("man")).unwrap();
        fs::write(root.path().join("man/foo.Rd"), "hand-written\n").unwrap();
        fs::write(root.path().join("NAMESPACE"), "hand-written\n").unwrap();
        let rd = rd_output([("man/foo.Rd", "new")]);
        let namespace_output = namespace("new namespace");
        let result = plan_outputs(root.path(), &rd, &namespace_output);
        let errors = result.expect_err("both outputs should be reported");
        assert_eq!(errors.errors.len(), 2);
    }

    #[test]
    fn invalid_utf8_is_a_read_error_and_is_not_overwritten() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("man")).unwrap();
        let path = root.path().join("man/foo.Rd");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        let rd = rd_output([("man/foo.Rd", "new")]);
        let namespace_output = namespace("namespace");
        let errors = plan_outputs(root.path(), &rd, &namespace_output)
            .expect_err("invalid UTF-8 must fail closed");
        assert_eq!(errors.errors.len(), 1);
        match &errors.errors[0] {
            OutputError::Io {
                operation: OutputOperation::Read,
                source,
                ..
            } => assert_eq!(source.kind(), std::io::ErrorKind::InvalidData),
            error => panic!("unexpected error: {error:?}"),
        }
        assert_eq!(fs::read(path).unwrap(), [0xff, 0xfe]);
    }

    #[test]
    fn plan_entries_are_sorted_by_relative_path() {
        let root = tempdir().unwrap();
        let rd = rd_output([("man/z.Rd", "z"), ("man/a.Rd", "a")]);
        let namespace_output = namespace("namespace");
        let output_plan = plan(root.path(), &rd, &namespace_output);
        let paths = output_plan
            .entries()
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("NAMESPACE"),
                PathBuf::from("man/a.Rd"),
                PathBuf::from("man/z.Rd")
            ]
        );
    }

    #[test]
    fn invalid_rd_paths_are_rejected() {
        for path in [
            "/absolute.Rd",
            "man/../foo.Rd",
            "man/nested/foo.Rd",
            "docs/foo.Rd",
            "man/foo.txt",
            "man/CON.Rd",
            // Windows resolves the device from the segment before the first
            // period, and dotted topic names are ordinary in R.
            "man/CON.default.Rd",
            "man/aux.print.Rd",
        ] {
            let root = tempdir().unwrap();
            has_error(
                plan_outputs(
                    root.path(),
                    &rd_output([(path, "content")]),
                    &namespace("namespace"),
                ),
                |error| matches!(error, OutputError::InvalidOutputPath { .. }),
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_destinations_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("man")).unwrap();
        let outside = root.path().join("outside.Rd");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, root.path().join("man/foo.Rd")).unwrap();
        has_error(
            plan_outputs(
                root.path(),
                &rd_output([("man/foo.Rd", "new")]),
                &namespace("namespace"),
            ),
            |error| matches!(error, OutputError::UnsafeOutputPath { .. }),
        );
    }

    /// A symlinked `man` would place the temporary file outside the package,
    /// so it is refused before the destination itself is considered.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_man_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().expect("directory outside the package");
        symlink(outside.path(), root.path().join("man")).unwrap();
        has_error(
            plan_outputs(
                root.path(),
                &rd_output([("man/foo.Rd", "new")]),
                &namespace("namespace"),
            ),
            |error| matches!(error, OutputError::UnsafeOutputPath { .. }),
        );
        assert!(!outside.path().join("foo.Rd").exists());
    }

    #[test]
    fn write_failure_reports_completed_and_not_attempted_paths() {
        let root = tempdir().unwrap();
        let rd = rd_output([("man/a.Rd", "a"), ("man/b.Rd", "b")]);
        let namespace = namespace("namespace");
        let output_plan = plan(root.path(), &rd, &namespace);
        fs::create_dir(root.path().join("man")).unwrap();
        fs::create_dir(root.path().join("man/a.Rd")).unwrap();
        let failure = apply_outputs(output_plan).expect_err("directory destination must fail");
        assert_eq!(failure.completed.created, vec![PathBuf::from("NAMESPACE")]);
        assert_eq!(failure.not_attempted, vec![PathBuf::from("man/b.Rd")]);
        assert!(matches!(
            failure.error,
            OutputError::UnsafeOutputPath { .. }
        ));
        assert_eq!(
            fs::read_to_string(root.path().join("NAMESPACE")).unwrap(),
            namespace.content
        );
    }

    #[test]
    fn output_error_keeps_operation_specific_io_context() {
        let root = tempdir().unwrap();
        let missing = root.path().join("missing");
        let errors = plan_outputs(&missing, &rd_output([]), &namespace("namespace"))
            .expect_err("missing root must fail");
        assert!(matches!(
            errors.errors.as_slice(),
            [OutputError::Io {
                operation: OutputOperation::Read,
                ..
            }]
        ));
    }

    #[test]
    fn namespace_only_output_does_not_create_man() {
        let root = tempdir().unwrap();
        let rd = rd_output([]);
        let namespace_output = namespace("namespace");
        let output_plan = plan(root.path(), &rd, &namespace_output);
        assert!(!root.path().join("man").exists());
        apply_outputs(output_plan).expect("namespace write succeeds");
        assert!(!root.path().join("man").exists());
    }

    #[test]
    fn output_kind_is_retained_in_the_plan() {
        let root = tempdir().unwrap();
        let rd = rd_output([("man/a.Rd", "a")]);
        let namespace_output = namespace("n");
        let output_plan = plan(root.path(), &rd, &namespace_output);
        assert_eq!(
            output_plan
                .entries()
                .find(|entry| entry.relative_path == Path::new("NAMESPACE"))
                .unwrap()
                .kind,
            OutputKind::Namespace
        );
        assert_eq!(
            output_plan
                .entries()
                .find(|entry| entry.relative_path == Path::new("man/a.Rd"))
                .unwrap()
                .kind,
            OutputKind::Rd
        );
    }
}
