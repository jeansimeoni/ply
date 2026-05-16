mod adapters;
mod config;
mod git;
mod ops;
mod ui;

use anyhow::{Result, anyhow};
use config::InitOptions;
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
        Command::Init(options) => {
            let options = options.resolve()?;
            let report = ops::init_project(project_root, options)?;
            let mut body = format!("Project root: {}", project_root.display());
            body.push_str("\n\nCreated:");
            if report.created_manifest {
                body.push('\n');
                body.push_str(&ui::list_item("ply.toml"));
            } else {
                body.push('\n');
                body.push_str(&ui::list_item("reused existing ply.toml"));
            }
            body.push('\n');
            body.push_str(&ui::list_item("prepared .ply/ local state"));
            if report.created_local_fixture {
                body.push('\n');
                body.push_str(&ui::list_item("scaffolded ply-packages/example-review"));
            }
            body.push_str("\n\nConfigured:");
            body.push('\n');
            body.push_str(&ui::list_item(if report.ignore_config {
                "Ply config is ignored locally via .git/info/exclude"
            } else {
                "Ply config remains trackable in the repository"
            }));
            ui::print_stdout(Tone::Success, "Initialized Ply project", &body);
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
        Command::Help(topic) => {
            print_help(topic);
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct Cli {
    command: Command,
}

#[derive(Debug, Clone)]
enum Command {
    Init(InitCli),
    Apply,
    Diff,
    Doctor,
    List,
    Sources,
    Adapters,
    Clean { yes: bool },
    Help(HelpTopic),
}

#[derive(Debug, Clone, Copy)]
enum HelpTopic {
    General,
    Init,
    Apply,
    Diff,
    Doctor,
    List,
    Sources,
    Adapters,
    Clean,
}

#[derive(Debug, Clone, Copy, Default)]
struct InitCli {
    scaffold_local_packages: Option<bool>,
    ignore_config: Option<bool>,
    yes: bool,
}

impl InitCli {
    fn resolve(self) -> Result<InitOptions> {
        let scaffold_local_packages = match self.scaffold_local_packages {
            Some(value) => value,
            None if self.yes => false,
            None => ui::prompt_yes_no(
                "Scaffold local package source",
                "Do you want Ply to create a local `ply-packages/` source in this project?\n\nChoose this when you want to bake packages directly into the repository.",
                false,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?,
        };

        let ignore_config = match self.ignore_config {
            Some(value) => value,
            None if self.yes => false,
            None => ui::prompt_yes_no(
                "Ignore Ply config locally",
                "Do you want all Ply files to stay ignored in this clone, including `ply.toml`, `ply.lock`, and `ply-packages/`?",
                false,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?,
        };

        Ok(InitOptions {
            scaffold_local_packages,
            ignore_config,
        })
    }
}

impl Command {
    fn requires_init(&self) -> bool {
        matches!(
            self,
            Self::Apply | Self::Diff | Self::Doctor | Self::List | Self::Sources
        )
    }
}

impl Cli {
    fn parse(args: Vec<String>) -> Result<Self> {
        let command = match args.first().map(String::as_str) {
            None | Some("-h") | Some("--help") => Command::Help(HelpTopic::General),
            Some("help") => Command::Help(parse_help_topic(&args[1..])?),
            Some("init") => parse_init_command(&args[1..])?,
            Some("apply") => parse_simple_command(&args[1..], Command::Apply, HelpTopic::Apply)?,
            Some("diff") => parse_simple_command(&args[1..], Command::Diff, HelpTopic::Diff)?,
            Some("doctor") => parse_simple_command(&args[1..], Command::Doctor, HelpTopic::Doctor)?,
            Some("list") => parse_simple_command(&args[1..], Command::List, HelpTopic::List)?,
            Some("sources") => {
                parse_simple_command(&args[1..], Command::Sources, HelpTopic::Sources)?
            }
            Some("adapters") => {
                parse_simple_command(&args[1..], Command::Adapters, HelpTopic::Adapters)?
            }
            Some("clean") | Some("nuke") => parse_clean_command(&args[1..])?,
            Some(other) => {
                return Err(anyhow!(
                    "unknown command `{other}`\n\n{}",
                    help_text(HelpTopic::General).trim_end()
                ));
            }
        };

        Ok(Self { command })
    }
}

fn parse_help_topic(args: &[String]) -> Result<HelpTopic> {
    match args {
        [] => Ok(HelpTopic::General),
        [topic] => match topic.as_str() {
            "init" => Ok(HelpTopic::Init),
            "apply" => Ok(HelpTopic::Apply),
            "diff" => Ok(HelpTopic::Diff),
            "doctor" => Ok(HelpTopic::Doctor),
            "list" => Ok(HelpTopic::List),
            "sources" => Ok(HelpTopic::Sources),
            "adapters" => Ok(HelpTopic::Adapters),
            "clean" | "nuke" => Ok(HelpTopic::Clean),
            other => Err(anyhow!("unknown help topic `{other}`")),
        },
        _ => Err(anyhow!("help accepts at most one command name")),
    }
}

fn parse_simple_command(args: &[String], command: Command, topic: HelpTopic) -> Result<Command> {
    match args {
        [] => Ok(command),
        [flag] if is_help_flag(flag) => Ok(Command::Help(topic)),
        [other, ..] => Err(anyhow!("unknown flag `{other}`")),
    }
}

fn parse_init_command(args: &[String]) -> Result<Command> {
    let mut cli = InitCli::default();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Init)),
            "--with-packages" => cli.scaffold_local_packages = Some(true),
            "--without-packages" => cli.scaffold_local_packages = Some(false),
            "--ignore-config" => cli.ignore_config = Some(true),
            "--track-config" => cli.ignore_config = Some(false),
            "--yes" | "-y" => cli.yes = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(Command::Init(cli))
}

fn parse_clean_command(args: &[String]) -> Result<Command> {
    let mut yes = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Clean)),
            "--yes" | "-y" => yes = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(Command::Clean { yes })
}

fn is_help_flag(flag: &str) -> bool {
    matches!(flag, "--help" | "-h")
}

fn print_help(topic: HelpTopic) {
    let title = match topic {
        HelpTopic::General => "Ply CLI",
        HelpTopic::Init => "ply init",
        HelpTopic::Apply => "ply apply",
        HelpTopic::Diff => "ply diff",
        HelpTopic::Doctor => "ply doctor",
        HelpTopic::List => "ply list",
        HelpTopic::Sources => "ply sources",
        HelpTopic::Adapters => "ply adapters",
        HelpTopic::Clean => "ply clean",
    };
    ui::print_stdout(Tone::Info, title, help_text(topic).trim_end());
}

fn help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::General => {
            r#"ply

Usage:
  ply <command>
  ply <command> --help

Commands:
  init       initialize Ply in the current project
  apply      resolve packages, render generated output, and expose managed assets
  diff       compare desired output with current generated/exposed state
  doctor     validate manifest, sources, package layout, and git safety
  list       show resolved packages
  sources    show configured sources and pinned revisions
  adapters   show supported adapters
  clean      remove Ply-managed files from this project
  nuke       alias for clean
  help       show this help or help for a specific command
"#
        }
        HelpTopic::Init => {
            r#"Usage:
  ply init [options]

Options:
  --with-packages     scaffold a local `ply-packages/` source
  --without-packages  do not create a local package source
  --ignore-config     keep `ply.toml`, `ply.lock`, `ply-packages/`, and `.ply/` ignored locally
  --track-config      keep Ply configuration trackable in the repository
  -y, --yes           skip prompts and accept defaults for unspecified options
  -h, --help          show this help
"#
        }
        HelpTopic::Apply => {
            r#"Usage:
  ply apply

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::Diff => {
            r#"Usage:
  ply diff

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::Doctor => {
            r#"Usage:
  ply doctor

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::List => {
            r#"Usage:
  ply list

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::Sources => {
            r#"Usage:
  ply sources

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::Adapters => {
            r#"Usage:
  ply adapters

Options:
  -h, --help          show this help
"#
        }
        HelpTopic::Clean => {
            r#"Usage:
  ply clean [options]
  ply nuke [options]

Options:
  -y, --yes           skip the destructive confirmation prompt
  -h, --help          show this help
"#
        }
    }
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
    fn parse_init_command_flags() -> Result<()> {
        let cli = Cli::parse(vec![
            "init".to_string(),
            "--with-packages".to_string(),
            "--ignore-config".to_string(),
            "--yes".to_string(),
        ])?;
        match cli.command {
            Command::Init(init) => {
                assert_eq!(init.scaffold_local_packages, Some(true));
                assert_eq!(init.ignore_config, Some(true));
                assert!(init.yes);
            }
            other => panic!("expected init command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_command_specific_help() -> Result<()> {
        let cli = Cli::parse(vec!["init".to_string(), "--help".to_string()])?;
        match cli.command {
            Command::Help(HelpTopic::Init) => Ok(()),
            other => panic!("expected init help, got {other:?}"),
        }
    }
}
