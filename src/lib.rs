mod adapters;
mod config;
mod git;
mod ops;
mod ui;

use anyhow::{Result, anyhow};
use adapters::AssetKind;
use config::InitOptions;
use ops::{ApplyOptions, CleanOptions, CommandTarget, InitRequest, PackageInitRequest};
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
            let request = options.resolve()?;
            let report = ops::init_project(project_root, request)?;
            let mut body = format!("Target root: {}", report.target_root.display());
            body.push_str("\n\nPlanned:");
            body.push('\n');
            body.push_str(&ui::list_item(if report.created_manifest {
                "create or reuse ply.toml"
            } else {
                "reuse existing ply.toml"
            }));
            body.push('\n');
            body.push_str(&ui::list_item("prepare .ply/ local state"));
            if report.created_local_fixture {
                body.push('\n');
                body.push_str(&ui::list_item("scaffold ply-packages/example-review"));
            }
            body.push_str("\n\nConfigured:");
            body.push('\n');
            body.push_str(&ui::list_item(if report.ignore_config {
                "Ply config is ignored locally when this root is a Git repo"
            } else {
                "Ply config remains trackable"
            }));
            let title = if report.dry_run {
                "Init dry-run"
            } else {
                "Initialized Ply"
            };
            ui::print_stdout(Tone::Success, title, &body);
        }
        Command::InitPackage(options) => {
            let request = options.resolve(&project_root)?;
            let report = ops::init_package(project_root, request)?;
            let mut body = format!("Target root: {}", report.target_root.display());
            body.push_str("\n\nPlanned:");
            for path in &report.created_paths {
                body.push('\n');
                body.push_str(&ui::list_item(&format!("create {}", path.display())));
            }
            let title = if report.dry_run {
                "Package init dry-run"
            } else {
                "Initialized Ply package"
            };
            ui::print_stdout(Tone::Success, title, &body);
        }
        Command::Apply(options) => {
            let report = ops::apply(project_root, options)?;
            let title = if report.dry_run {
                "Apply dry-run"
            } else {
                "Applied managed assets"
            };
            let tone = if report.dry_run {
                Tone::Info
            } else {
                Tone::Success
            };
            ui::print_stdout(tone, title, &report.body);
        }
        Command::Diff => {
            let summary = ops::diff(project_root)?;
            let tone = if summary == "no differences" {
                Tone::Success
            } else {
                Tone::Info
            };
            let body = if summary == "no differences" {
                "Managed files match the resolved project and global layers."
            } else {
                &summary
            };
            ui::print_stdout(tone, "Diff report", body);
        }
        Command::Doctor { target } => {
            let summary = ops::doctor(project_root, target)?;
            ui::print_stdout(Tone::Info, "Doctor report", &summary);
        }
        Command::List { target } => {
            let summary = ops::list_packages(project_root, target)?;
            ui::print_stdout(Tone::Info, "Resolved packages", &summary);
        }
        Command::Sources { target } => {
            let summary = ops::list_sources(project_root, target)?;
            ui::print_stdout(Tone::Info, "Resolved sources", &summary);
        }
        Command::Adapters => {
            ui::print_stdout(
                Tone::Info,
                "Supported adapters",
                &adapters::adapter_summary(),
            );
        }
        Command::Clean(options) => {
            let target = if options.global {
                CommandTarget::Global
            } else {
                CommandTarget::Project
            };
            let preview = ops::preview_cleanup(
                project_root,
                CleanOptions {
                    dry_run: options.dry_run,
                    target,
                },
            )?;
            let title_target = match target {
                CommandTarget::Project => "this project",
                CommandTarget::Global => "the user-global Ply layer",
            };
            if !options.yes && !options.dry_run {
                let mut body = format!(
                    "This will remove Ply-managed files and local state from {title_target}.\n"
                );
                for item in &preview.items {
                    body.push('\n');
                    body.push_str(&ui::list_item(item));
                }
                if preview.updates_git_excludes {
                    body.push('\n');
                    body.push_str(&ui::list_item("update .git/info/exclude"));
                }
                let confirmed = ui::prompt_confirmation("Remove Ply-managed files", &body)
                    .map_err(|err| anyhow!("failed to read confirmation: {err}"))?;
                if !confirmed {
                    ui::print_stdout(Tone::Info, "Cancelled cleanup", "No files were removed.");
                    return Ok(());
                }
            }

            let report = ops::clean_project(
                project_root,
                CleanOptions {
                    dry_run: options.dry_run,
                    target,
                },
            )?;
            let mut body = String::new();
            if report.removed_items.is_empty() {
                body.push_str("No Ply-managed files were found.");
            } else {
                body.push_str(if options.dry_run {
                    "Would remove:"
                } else {
                    "Removed:"
                });
                for item in &report.removed_items {
                    body.push('\n');
                    body.push_str(&ui::list_item(item));
                }
            }
            if report.updated_git_excludes {
                body.push_str("\n\nUpdated:");
                body.push('\n');
                body.push_str(&ui::list_item(if options.dry_run {
                    "would remove the Ply block from .git/info/exclude"
                } else {
                    "removed the Ply block from .git/info/exclude"
                }));
            }
            let title = if options.dry_run {
                "Cleanup dry-run"
            } else {
                "Removed Ply-managed files"
            };
            ui::print_stdout(Tone::Warning, title, &body);
        }
        Command::Help(topic) => print_help(topic),
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
    InitPackage(InitPackageCli),
    Apply(ApplyOptions),
    Diff,
    Doctor { target: CommandTarget },
    List { target: CommandTarget },
    Sources { target: CommandTarget },
    Adapters,
    Clean(CleanCli),
    Help(HelpTopic),
}

#[derive(Debug, Clone, Copy)]
enum HelpTopic {
    General,
    Init,
    InitPackage,
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
    global: bool,
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct CleanCli {
    yes: bool,
    global: bool,
    dry_run: bool,
}

#[derive(Debug, Clone, Default)]
struct InitPackageCli {
    name: Option<String>,
    path: Option<String>,
    kinds: Vec<AssetKind>,
    dry_run: bool,
}

impl InitCli {
    fn resolve(self) -> Result<InitRequest> {
        let scaffold_local_packages = match self.scaffold_local_packages {
            Some(value) => value,
            None if self.yes => false,
            None => ui::prompt_yes_no(
                "Scaffold local package source",
                "Do you want Ply to create a local `ply-packages/` source in this target?\n\nChoose this when you want to bake packages directly into the target root.",
                "Create local package source",
                false,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?,
        };

        let ignore_config = match self.ignore_config {
            Some(value) => value,
            None if self.yes => false,
            None => ui::prompt_yes_no(
                "Ignore Ply config locally",
                "Do you want all Ply files to stay ignored in this target when it is a Git repo, including `ply.toml`, `ply.lock`, and `ply-packages/`?",
                "Keep Ply config ignored locally",
                false,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?,
        };

        Ok(InitRequest {
            options: InitOptions {
                scaffold_local_packages,
                ignore_config,
            },
            dry_run: self.dry_run,
            target: if self.global {
                CommandTarget::Global
            } else {
                CommandTarget::Project
            },
        })
    }
}

impl InitPackageCli {
    fn resolve(self, project_root: &Path) -> Result<PackageInitRequest> {
        let name = self
            .name
            .ok_or_else(|| anyhow!("`ply init package` requires `--name`"))?;
        let path = self
            .path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| project_root.to_path_buf());
        Ok(PackageInitRequest {
            name,
            path,
            kinds: self.kinds,
            dry_run: self.dry_run,
        })
    }
}

impl Command {
    fn requires_init(&self) -> bool {
        matches!(
            self,
            Self::Apply(_)
                | Self::Diff
                | Self::Doctor {
                    target: CommandTarget::Project
                }
                | Self::List {
                    target: CommandTarget::Project
                }
                | Self::Sources {
                    target: CommandTarget::Project
                }
        )
    }
}

impl Cli {
    fn parse(args: Vec<String>) -> Result<Self> {
        let command = match args.first().map(String::as_str) {
            None | Some("-h") | Some("--help") => Command::Help(HelpTopic::General),
            Some("help") => Command::Help(parse_help_topic(&args[1..])?),
            Some("init") => parse_init_command(&args[1..])?,
            Some("apply") => parse_apply_command(&args[1..])?,
            Some("diff") => parse_diff_command(&args[1..])?,
            Some("doctor") => parse_targeted_command(&args[1..], "doctor")?,
            Some("list") => parse_targeted_command(&args[1..], "list")?,
            Some("sources") => parse_targeted_command(&args[1..], "sources")?,
            Some("adapters") => {
                parse_simple_help_command(&args[1..], Command::Adapters, HelpTopic::Adapters)?
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
            "init-package" => Ok(HelpTopic::InitPackage),
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

fn parse_simple_help_command(
    args: &[String],
    command: Command,
    topic: HelpTopic,
) -> Result<Command> {
    match args {
        [] => Ok(command),
        [flag] if is_help_flag(flag) => Ok(Command::Help(topic)),
        [other, ..] => Err(anyhow!("unknown flag `{other}`")),
    }
}

fn parse_init_command(args: &[String]) -> Result<Command> {
    if let Some(subcommand) = args.first() {
        if subcommand == "package" {
            return parse_init_package_command(&args[1..]);
        }
    }
    let mut cli = InitCli::default();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Init)),
            "--with-packages" => cli.scaffold_local_packages = Some(true),
            "--without-packages" => cli.scaffold_local_packages = Some(false),
            "--ignore-config" => cli.ignore_config = Some(true),
            "--track-config" => cli.ignore_config = Some(false),
            "--yes" | "-y" => cli.yes = true,
            "--global" | "-g" => cli.global = true,
            "--dry-run" => cli.dry_run = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(Command::Init(cli))
}

fn parse_init_package_command(args: &[String]) -> Result<Command> {
    let mut cli = InitPackageCli::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::InitPackage)),
            "--name" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("missing value for `--name`"))?;
                cli.name = Some(value.clone());
            }
            "--path" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("missing value for `--path`"))?;
                cli.path = Some(value.clone());
            }
            "--kinds" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("missing value for `--kinds`"))?;
                cli.kinds = value
                    .split(',')
                    .map(|item| AssetKind::parse(item.trim()))
                    .collect::<Result<Vec<_>>>()?;
            }
            "--dry-run" => cli.dry_run = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
        index += 1;
    }
    Ok(Command::InitPackage(cli))
}

fn parse_apply_command(args: &[String]) -> Result<Command> {
    let mut options = ApplyOptions {
        dry_run: false,
        yes: false,
    };
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Apply)),
            "--dry-run" => options.dry_run = true,
            "--yes" | "-y" => options.yes = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(Command::Apply(options))
}

fn parse_diff_command(args: &[String]) -> Result<Command> {
    match args {
        [] => Ok(Command::Diff),
        [flag] if is_help_flag(flag) => Ok(Command::Help(HelpTopic::Diff)),
        [other, ..] => Err(anyhow!("unknown flag `{other}`")),
    }
}

fn parse_targeted_command(args: &[String], name: &str) -> Result<Command> {
    let mut target = CommandTarget::Project;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                return Ok(Command::Help(match name {
                    "doctor" => HelpTopic::Doctor,
                    "list" => HelpTopic::List,
                    "sources" => HelpTopic::Sources,
                    _ => unreachable!(),
                }));
            }
            "--global" | "-g" => target = CommandTarget::Global,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(match name {
        "doctor" => Command::Doctor { target },
        "list" => Command::List { target },
        "sources" => Command::Sources { target },
        _ => unreachable!(),
    })
}

fn parse_clean_command(args: &[String]) -> Result<Command> {
    let mut options = CleanCli::default();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help(HelpTopic::Clean)),
            "--yes" | "-y" => options.yes = true,
            "--global" | "-g" => options.global = true,
            "--dry-run" => options.dry_run = true,
            other => return Err(anyhow!("unknown flag `{other}`")),
        }
    }
    Ok(Command::Clean(options))
}

fn is_help_flag(flag: &str) -> bool {
    matches!(flag, "--help" | "-h")
}

fn print_help(topic: HelpTopic) {
    let title = match topic {
        HelpTopic::General => "Ply CLI",
        HelpTopic::Init => "ply init",
        HelpTopic::InitPackage => "ply init package",
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
  init       initialize Ply in the project, global root, or a package root
  apply      resolve packages, preview or write managed assets
  diff       show managed content drift with layer origin context
  doctor     validate manifest, sources, package layout, and git safety
  list       show resolved packages
  sources    show configured sources and pinned revisions
  adapters   show supported adapters
  clean      remove Ply-managed files from the project or global root
  nuke       alias for clean
  help       show this help or help for a specific command
"#
        }
        HelpTopic::Init => {
            r#"Usage:
  ply init [options]
  ply init package [options]

Options:
  --with-packages     scaffold a local `ply-packages/` source
  --without-packages  do not create a local package source
  --ignore-config     keep Ply config ignored locally when the target is a Git repo
  --track-config      keep Ply configuration trackable
  --global, -g        target the user-global Ply root
  --dry-run           preview what init would create
  -y, --yes           skip prompts and accept defaults for unspecified options
  -h, --help          show this help
"#
        }
        HelpTopic::InitPackage => {
            r#"Usage:
  ply init package [options]

Options:
  --name <name>       package name written into ply-package.toml
  --path <dir>        target package root; defaults to the current directory
  --kinds <list>      comma-separated asset kinds to scaffold
  --dry-run           preview what package init would create
  -h, --help          show this help
"#
        }
        HelpTopic::Apply => {
            r#"Usage:
  ply apply [options]

Options:
  --dry-run           preview layering results, planned assets, and drift consent needs
  -y, --yes           overwrite drifted managed exposed files without prompting
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
  ply doctor [options]

Options:
  --global, -g        inspect the user-global Ply root
  -h, --help          show this help
"#
        }
        HelpTopic::List => {
            r#"Usage:
  ply list [options]

Options:
  --global, -g        inspect the user-global Ply root
  -h, --help          show this help
"#
        }
        HelpTopic::Sources => {
            r#"Usage:
  ply sources [options]

Options:
  --global, -g        inspect the user-global Ply root
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
  --global, -g        target the user-global Ply root
  --dry-run           preview removals without deleting anything
  -y, --yes           skip the destructive confirmation prompt
  -h, --help          show this help
"#
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AssetKind;
    use tempfile::TempDir;

    #[test]
    fn commands_that_require_init_return_ply_error() -> Result<()> {
        let temp = TempDir::new()?;
        for command in [
            Command::Apply(ApplyOptions {
                dry_run: false,
                yes: false,
            }),
            Command::Diff,
            Command::Doctor {
                target: CommandTarget::Project,
            },
            Command::List {
                target: CommandTarget::Project,
            },
            Command::Sources {
                target: CommandTarget::Project,
            },
        ] {
            let err = run_command(temp.path(), command).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("ply is not initialized"));
        }
        Ok(())
    }

    #[test]
    fn parse_apply_flags() -> Result<()> {
        let cli = Cli::parse(vec![
            "apply".to_string(),
            "--dry-run".to_string(),
            "--yes".to_string(),
        ])?;
        match cli.command {
            Command::Apply(options) => {
                assert!(options.dry_run);
                assert!(options.yes);
            }
            other => panic!("expected apply command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_global_clean_flags() -> Result<()> {
        let cli = Cli::parse(vec![
            "clean".to_string(),
            "-g".to_string(),
            "--dry-run".to_string(),
        ])?;
        match cli.command {
            Command::Clean(options) => {
                assert!(options.global);
                assert!(options.dry_run);
            }
            other => panic!("expected clean command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_init_package_flags() -> Result<()> {
        let cli = Cli::parse(vec![
            "init".to_string(),
            "package".to_string(),
            "--name".to_string(),
            "review-tools".to_string(),
            "--path".to_string(),
            "./packages/review-tools".to_string(),
            "--kinds".to_string(),
            "skills,commands".to_string(),
            "--dry-run".to_string(),
        ])?;
        match cli.command {
            Command::InitPackage(options) => {
                assert_eq!(options.name.as_deref(), Some("review-tools"));
                assert_eq!(options.path.as_deref(), Some("./packages/review-tools"));
                assert_eq!(options.kinds, vec![AssetKind::Skills, AssetKind::Commands]);
                assert!(options.dry_run);
            }
            other => panic!("expected init package command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_init_package_agents_kind() -> Result<()> {
        let cli = Cli::parse(vec![
            "init".to_string(),
            "package".to_string(),
            "--name".to_string(),
            "review-tools".to_string(),
            "--kinds".to_string(),
            "agents".to_string(),
        ])?;
        match cli.command {
            Command::InitPackage(options) => {
                assert_eq!(options.kinds, vec![AssetKind::Agents]);
            }
            other => panic!("expected init package command, got {other:?}"),
        }
        Ok(())
    }
}
