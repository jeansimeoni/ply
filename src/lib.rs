mod adapters;
mod config;
mod git;
mod ops;

use anyhow::{Result, anyhow};
use std::env;
use std::path::Path;

pub fn run() -> Result<()> {
    let cli = Cli::parse(env::args().skip(1).collect())?;
    let project_root = env::current_dir()?;
    run_command(&project_root, cli.command)
}

fn run_command(project_root: &Path, command: Command) -> Result<()> {
    if command.requires_init() {
        config::ensure_initialized(project_root)?;
    }
    match command {
        Command::Init => {
            ops::init_project(project_root)?;
            println!("initialized ply project in {}", project_root.display());
        }
        Command::Apply => {
            let summary = ops::apply(project_root)?;
            println!("{summary}");
        }
        Command::Diff => {
            let summary = ops::diff(project_root)?;
            println!("{summary}");
        }
        Command::Doctor => {
            let summary = ops::doctor(project_root)?;
            println!("{summary}");
        }
        Command::List => {
            let summary = ops::list_packages(project_root)?;
            println!("{summary}");
        }
        Command::Sources => {
            let summary = ops::list_sources(project_root)?;
            println!("{summary}");
        }
        Command::Adapters => {
            println!("{}", adapters::adapter_summary());
        }
        Command::Help => {
            print_help();
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Cli {
    command: Command,
}

#[derive(Debug, Clone, Copy)]
enum Command {
    Init,
    Apply,
    Diff,
    Doctor,
    List,
    Sources,
    Adapters,
    Help,
}

impl Command {
    fn requires_init(self) -> bool {
        matches!(
            self,
            Self::Apply | Self::Diff | Self::Doctor | Self::List | Self::Sources
        )
    }
}

impl Cli {
    fn parse(args: Vec<String>) -> Result<Self> {
        let command = match args.first().map(String::as_str) {
            None | Some("-h") | Some("--help") | Some("help") => Command::Help,
            Some("init") => Command::Init,
            Some("apply") => Command::Apply,
            Some("diff") => Command::Diff,
            Some("doctor") => Command::Doctor,
            Some("list") => Command::List,
            Some("sources") => Command::Sources,
            Some("adapters") => Command::Adapters,
            Some(other) => {
                return Err(anyhow!(
                    "unknown command `{other}`\n\n{}",
                    help_text().trim_end()
                ));
            }
        };

        Ok(Self { command })
    }
}

fn print_help() {
    print!("{}", help_text());
}

fn help_text() -> &'static str {
    r#"ply

Usage:
  ply <command>

Commands:
  init       scaffold ply.toml and local state files
  apply      resolve packages, render generated output, and expose managed assets
  diff       compare desired output with current generated/exposed state
  doctor     validate manifest, sources, package layout, and git safety
  list       show resolved packages
  sources    show configured sources and pinned revisions
  adapters   show supported adapters
  help       show this help
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn commands_that_require_init_return_ply_error() -> Result<()> {
        let temp = TempDir::new()?;

        for command in [
            Command::Apply,
            Command::Diff,
            Command::Doctor,
            Command::List,
            Command::Sources,
        ] {
            let err = run_command(temp.path(), command).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("ply is not initialized"));
            assert!(message.contains("ply init"));
        }

        Ok(())
    }
}
