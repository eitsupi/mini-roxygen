use std::ffi::OsStr;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "roxy")]
#[command(about = "Generate R package documentation from roxygen comments")]
#[command(version)]
#[command(arg_required_else_help = true)]
pub(crate) struct Args {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate man/*.Rd and NAMESPACE for a package
    Doc(DocArgs),
}

#[derive(Debug, ClapArgs)]
pub(crate) struct DocArgs {
    /// The package directory to document
    #[arg(default_value = ".")]
    pub(crate) package_path: PathBuf,
    /// An R library path from which installed package metadata may be read.
    #[arg(
        long = "r-lib-path",
        value_name = "PATH",
        conflicts_with = "r_lib_paths_list"
    )]
    pub(crate) r_lib_paths: Vec<PathBuf>,
    /// An OS-native path-list string of R library paths.
    #[arg(
        long = "r-lib-paths",
        value_name = "PATH_LIST",
        value_parser = parse_r_lib_path_list,
        conflicts_with = "r_lib_paths"
    )]
    pub(crate) r_lib_paths_list: Option<RLibPathList>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RLibPathList(Vec<PathBuf>);

fn parse_r_lib_path_list(value: &str) -> Result<RLibPathList, String> {
    let paths: Vec<PathBuf> = std::env::split_paths(OsStr::new(value)).collect();
    if paths.is_empty() {
        return Err("R library path list must not be empty".to_owned());
    }
    if paths.iter().any(|path| path.as_os_str().is_empty()) {
        return Err("R library path list must not contain empty components".to_owned());
    }
    Ok(RLibPathList(paths))
}

impl DocArgs {
    pub(crate) fn effective_r_lib_paths(&self) -> Vec<PathBuf> {
        self.r_lib_paths_list
            .as_ref()
            .map(|paths| paths.0.clone())
            .unwrap_or_else(|| self.r_lib_paths.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Args, Command};

    #[test]
    fn doc_defaults_to_the_current_directory() {
        let args = Args::try_parse_from(["roxy", "doc"]).expect("arguments should parse");
        let Command::Doc(doc) = args.command;
        assert_eq!(doc.package_path, std::path::Path::new("."));
        assert!(doc.r_lib_paths.is_empty());
        assert!(doc.r_lib_paths_list.is_none());
    }

    #[test]
    fn doc_accepts_one_explicit_path() {
        let args = Args::try_parse_from(["roxy", "doc", "pkg"]).expect("arguments should parse");
        let Command::Doc(doc) = args.command;
        assert_eq!(doc.package_path, std::path::Path::new("pkg"));
        assert!(doc.r_lib_paths.is_empty());
        assert!(doc.r_lib_paths_list.is_none());
    }

    #[test]
    fn invalid_commands_are_usage_errors() {
        assert!(Args::try_parse_from(["roxy"]).is_err());
        assert!(Args::try_parse_from(["roxy", "check"]).is_err());
        // The long spelling is not an alias: only `doc` is accepted.
        assert!(Args::try_parse_from(["roxy", "document"]).is_err());
    }

    #[test]
    fn doc_accepts_repeated_r_library_paths_in_order() {
        let args = Args::try_parse_from([
            "roxy",
            "doc",
            "--r-lib-path",
            "first",
            "pkg",
            "--r-lib-path",
            "second",
        ])
        .expect("arguments should parse");
        let Command::Doc(doc) = args.command;
        assert_eq!(doc.package_path, std::path::Path::new("pkg"));
        assert_eq!(
            doc.r_lib_paths,
            vec![
                std::path::PathBuf::from("first"),
                std::path::PathBuf::from("second")
            ]
        );
        assert!(doc.r_lib_paths_list.is_none());
    }

    #[test]
    fn doc_accepts_an_exact_path_containing_spaces() {
        let args = Args::try_parse_from(["roxy", "doc", "--r-lib-path", "/tmp/R library", "pkg"])
            .expect("arguments should parse");
        let Command::Doc(doc) = args.command;
        assert_eq!(doc.r_lib_paths, vec![PathBuf::from("/tmp/R library")]);
    }

    #[test]
    fn doc_expands_a_native_r_library_path_list_in_order() {
        let paths = [PathBuf::from("first"), PathBuf::from("second")];
        let path_list = std::env::join_paths(paths.iter()).expect("paths should join");
        let args = Args::try_parse_from([
            std::ffi::OsString::from("roxy"),
            std::ffi::OsString::from("doc"),
            std::ffi::OsString::from("--r-lib-paths"),
            path_list,
            std::ffi::OsString::from("pkg"),
        ])
        .expect("arguments should parse");
        let Command::Doc(doc) = args.command;
        assert_eq!(doc.r_lib_paths, Vec::<PathBuf>::new());
        assert_eq!(
            doc.effective_r_lib_paths(),
            vec![PathBuf::from("first"), PathBuf::from("second")]
        );
    }

    #[test]
    fn doc_rejects_empty_native_r_library_path_components() {
        let empty_component = std::env::join_paths([PathBuf::new(), PathBuf::from("first")])
            .expect("paths should join");
        let args = vec![
            std::ffi::OsString::from("roxy"),
            std::ffi::OsString::from("doc"),
            std::ffi::OsString::from("--r-lib-paths"),
            empty_component,
        ];
        let error = Args::try_parse_from(args)
            .expect_err("an empty path-list component should be rejected");
        assert!(error.to_string().contains("empty components"));

        let error = Args::try_parse_from(["roxy", "doc", "--r-lib-paths", ""])
            .expect_err("an empty path list should be rejected");
        assert!(error.to_string().contains("empty components"));
    }

    #[test]
    fn doc_rejects_conflicting_r_library_path_forms() {
        let error = Args::try_parse_from([
            "roxy",
            "doc",
            "--r-lib-path",
            "first",
            "--r-lib-paths",
            "second",
        ])
        .expect_err("the two path forms should conflict");
        assert!(error.to_string().contains("cannot be used with"));
    }
}
