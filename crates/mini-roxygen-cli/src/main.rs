use std::process::ExitCode;

mod args;
mod base_catalog;
mod config;
mod diagnostic;
mod doc;
mod documentation;
mod installed;
mod output;
mod provider;

fn main() -> ExitCode {
    let args = <args::Args as clap::Parser>::parse();
    let status = match args.command {
        args::Command::Doc(doc) => doc::run(doc),
    };
    ExitCode::from(status.exit_code())
}
