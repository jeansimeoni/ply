mod adapters;
mod config;
mod git;
mod ops;
mod ui;

use anyhow::{Result, anyhow};
use std::env;
use std::path::Path;
use ui::Tone;

pub fn run() -> Result<()> {
    let cli = Cli::parse(env::args().skip(1).collect())?;
    let project_root = env::current_dir()?;
    run_command(&project_root, cli.command)
}

pub fn print_error(err: &anyhow::Error) {
    ui::print_stderr(Tone::Error, "Ply failed", &ui::error_body(err));
}

fn run_command(project_root: &Path, command: Command) -> Result<()> {
    if command.requires_init() {
        config::ensure_initialized(project_root)?;
    }
    match command {
        Command::Init => {
            ops::init_project(project_root)?;
            ui::print_stdout(
                Tone::Success,
                "Initialized Ply project",
                &format!(
                    "Project root: {}\n\n{}\n{}\n{}",
                    project_root.display(),
                    ui::list_item("Created ply.toml"),
                    ui::list_item("Prepared .ply/ local state"),
                    ui::list_item("Updated .git/info/exclude with Ply-managed paths"),
                ),
            );
        }
        Command::Apply => {
            let summary = ops::apply(project_root)?;
            ui::print_stdout(Tone::Success, "Applied managed assets", &summary);
        }
        Command::Diff => {
            let summary = ops::diff(project_root)?;
            let tone = if summary == "no differences" {
                Tone::Success
            } else {
                Tone::Info
            };
            let body = if summary == "no differences" {
                "Generated and exposed files already match the current manifest."
            } else {
                &summary
            };
            ui::print_stdout(tone, "Diff report", body);
        }
        Command::Doctor => {
            let summary = ops::doctor(project_root)?;
            ui::print_stdout(Tone::Info, "Doctor report", &summary);
        }
        Command::List => {
            let summary = ops::list_packages(project_root)?;
            ui::print_stdout(Tone::Info, "Resolved packages", &summary);
        }
        Command::Sources => {
            let summary = ops::list_sources(project_root)?;
            ui::print_stdout(Tone::Info, "Resolved sources", &summary);
        }
        Command::Adapters => {
            ui::print_stdout(
                Tone::Info,
                "Supported adapters",
                &adapters::adapter_summary(),
            );
        }
        Command::Clean { yes } => {
            let preview = ops::preview_cleanup(project_root)?;
            if !yes {
                let mut body = String::from(
                    "This will remove Ply-managed files and local state from this project.\n",
                );
                for item in &preview.items {
                    body.push('\n');
                    body.push_str(&ui::list_item(item));
                }
                if preview.updates_git_excludes {
                    body.push('\n');
                    body.push_str(&ui::list_item("update .git/info/exclude"));
                }
                let confirmed = ui::prompt_confirmation("Remove Ply from this project", &body)
                    .map_err(|err| anyhow!("failed to read confirmation: {err}"))?;
                if !confirmed {
                    ui::print_stdout(Tone::Info, "Cancelled cleanup", "No files were removed.");
                    return Ok(());
                }
            }

            let report = ops::clean_project(project_root)?;
            let mut body = String::new();
            if report.removed_items.is_empty() {
                body.push_str("No Ply-managed files were removed.");
            } else {
                body.push_str("Removed:");
                for item in &report.removed_items {
                    body.push('\n');
                    body.push_str(&ui::list_item(item));
                }
            }
            if report.updated_git_excludes {
                body.push_str("\n\nUpdated:");
                body.push('\n');
                body.push_str(&ui::list_item(
                    "removed the Ply block from .git/info/exclude",
                ));
            }
            ui::print_stdout(Tone::Warning, "Removed Ply from this project", &body);
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
    Clean { yes: bool },
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
            Some("clean") | Some("nuke") => Command::Clean {
                yes: parse_yes_flag(&args[1..])?,
            },
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

fn parse_yes_flag(flags: &[String]) -> Result<bool> {
    let mut yes = false;
    for flag in flags {
        match flag.as_str() {
            "--yes" | "-y" => yes = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(yes)
}

fn print_help() {
    ui::print_stdout(Tone::Info, "Ply CLI", help_text().trim_end());
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
  clean      remove Ply-managed files from this project
  nuke       alias for clean
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

    #[test]
    fn parse_clean_command_alias_and_yes_flag() -> Result<()> {
        let cli = Cli::parse(vec!["nuke".to_string(), "--yes".to_string()])?;
        match cli.command {
            Command::Clean { yes } => assert!(yes),
            other => panic!("expected clean command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_clean_rejects_unknown_flags() {
        let err = parse_yes_flag(&["--force".to_string()]).unwrap_err();
        assert!(err.to_string().contains("unknown flag `--force`"));
    }
}
