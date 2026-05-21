mod adapters;
mod config;
mod git;
mod ops;
mod prompt_resources;
mod ui;

use adapters::AssetKind;
use anyhow::{Result, anyhow};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use config::InitOptions;
use ops::{
    AddSourceLocation, AddSourceRequest, AddSourceSshMode, ApplyOptions, CleanOptions,
    CommandTarget, InitRequest, PackageInitRequest, RemoveSourceRequest, UpdateSourcesRequest,
};
use std::env;
use std::path::Path;
use ui::Tone;

pub fn run() -> Result<()> {
    let Some(cli) = Cli::parse(env::args().skip(1).collect())? else {
        return Ok(());
    };
    let project_root = env::current_dir()?;
    match cli.command {
        Some(command) => run_command(&project_root, command),
        None => print_help(None),
    }
}

pub fn print_error(err: &anyhow::Error) {
    ui::print_stderr(Tone::Error, "Ply failed", &ui::error_body(err));
}

fn run_command(project_root: &Path, command: Command) -> Result<()> {
    if let Some(target) = command.init_target() {
        let root = resolve_target_root(project_root, target)?;
        let hint = match target {
            CommandTarget::Project => "ply init",
            CommandTarget::Global => "ply init -g",
        };
        config::ensure_initialized_with_hint(&root, hint)?;
    }

    match command {
        Command::Init(InitCommand {
            command: Some(InitSubcommand::Package(options)),
            ..
        }) => {
            let request = options.resolve(project_root)?;
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
        Command::Init(options) => {
            let request = options.options.resolve()?;
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
            body.push('\n');
            body.push_str(&ui::list_item(&format!(
                "enable adapters: {}",
                report.adapters.join(", ")
            )));
            let title = if report.dry_run {
                "Init dry-run"
            } else {
                "Initialized Ply"
            };
            ui::print_stdout(Tone::Success, title, &body);
        }
        Command::Apply(options) => {
            let report = ops::apply(project_root, options.into())?;
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
        Command::Doctor(target) => {
            let summary = ops::doctor(project_root, command_target(target.global))?;
            ui::print_stdout(Tone::Info, "Doctor report", &summary);
        }
        Command::List(target) => {
            let summary = ops::list_packages(project_root, command_target(target.global))?;
            ui::print_stdout(Tone::Info, "Resolved packages", &summary);
        }
        Command::Sources(target) => {
            let summary = ops::list_sources(project_root, command_target(target.global))?;
            ui::print_stdout(Tone::Info, "Resolved sources", &summary);
        }
        Command::Adapters => {
            ui::print_stdout(
                Tone::Info,
                "Supported adapters",
                &adapters::adapter_summary(),
            );
        }
        Command::Add(options) => {
            let target = command_target(options.global);
            let root = resolve_target_root(project_root, target)?;
            let ssh = match (&options.ssh, &options.ssh_key) {
                (true, None) => AddSourceSshMode::DefaultKey,
                (false, Some(path)) => AddSourceSshMode::KeyPath(path.clone()),
                _ => AddSourceSshMode::None,
            };
            let location = match (options.path, options.git) {
                (Some(path), None) => AddSourceLocation::Path(path),
                (None, Some(repo)) => AddSourceLocation::Git {
                    repo,
                    rev: options.rev,
                },
                _ => {
                    return Err(anyhow!(
                        "`ply add` requires exactly one of `--path` or `--git`"
                    ));
                }
            };
            let report = ops::add_source(
                &root,
                AddSourceRequest {
                    id: options.id,
                    location,
                    ssh,
                },
                target,
            )?;
            ui::print_stdout(Tone::Success, "Updated manifest", &report.body);
        }
        Command::Remove(options) => {
            let target = command_target(options.global);
            let root = resolve_target_root(project_root, target)?;
            let report = ops::remove_source(
                &root,
                RemoveSourceRequest {
                    id: options.source_id,
                    force: options.force,
                },
            )?;
            ui::print_stdout(Tone::Warning, "Updated manifest", &report.body);
        }
        Command::Update(options) => {
            let target = command_target(options.global);
            let report = ops::update_sources(
                project_root,
                UpdateSourcesRequest {
                    source_id: options.source_id,
                },
                target,
            )?;
            ui::print_stdout(Tone::Success, "Updated sources", &report.body);
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
        Command::Help(topic) => print_help(topic.topic)?,
    }

    Ok(())
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "ply",
    about = "Composable package manager for coding-agent assets",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    #[command(about = "initialize Ply in the project, global root, or a package root")]
    Init(InitCommand),
    #[command(about = "resolve sources, preview or write managed assets")]
    Apply(ApplyCli),
    #[command(about = "show managed content drift with layer origin context")]
    Diff,
    #[command(about = "validate manifest, sources, package roots, and git safety")]
    Doctor(TargetCli),
    #[command(about = "show resolved package roots")]
    List(TargetCli),
    #[command(about = "show configured sources and pinned revisions")]
    Sources(TargetCli),
    #[command(about = "show supported adapters")]
    Adapters,
    #[command(about = "add a source to ply.toml")]
    Add(AddCli),
    #[command(about = "remove a source from ply.toml")]
    Remove(RemoveCli),
    #[command(about = "refresh Git sources and rewrite ply.lock")]
    Update(UpdateCli),
    #[command(
        alias = "nuke",
        about = "remove Ply-managed files from the project or global root"
    )]
    Clean(CleanCli),
    #[command(about = "show this help or help for a specific command")]
    Help(HelpCli),
}

#[derive(Debug, Clone, Args)]
struct InitCommand {
    #[command(subcommand)]
    command: Option<InitSubcommand>,
    #[command(flatten)]
    options: InitCli,
}

#[derive(Debug, Clone, Subcommand)]
enum InitSubcommand {
    #[command(name = "package", about = "initialize one reusable package root")]
    Package(InitPackageCli),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum HelpTopic {
    Init,
    InitPackage,
    Apply,
    Diff,
    Doctor,
    List,
    Sources,
    Adapters,
    Add,
    Remove,
    Update,
    Clean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InitAdapter {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Args, Default)]
struct InitCli {
    #[arg(long = "with-packages", conflicts_with = "without_packages")]
    with_packages: bool,
    #[arg(long = "without-packages")]
    without_packages: bool,
    #[arg(long = "ignore-config", conflicts_with = "track_config")]
    ignore_config: bool,
    #[arg(long = "track-config")]
    track_config: bool,
    #[arg(long, value_delimiter = ',')]
    adapters: Vec<InitAdapter>,
    #[arg(long, short = 'y')]
    yes: bool,
    #[arg(long, short = 'g')]
    global: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct CleanCli {
    #[arg(long, short = 'y')]
    yes: bool,
    #[arg(long, short = 'g')]
    global: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct InitPackageCli {
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    path: Option<String>,
    #[arg(long, value_delimiter = ',')]
    kinds: Vec<String>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Args)]
struct ApplyCli {
    #[arg(long)]
    dry_run: bool,
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Debug, Clone, Args, Default)]
struct TargetCli {
    #[arg(long, short = 'g')]
    global: bool,
}

#[derive(Debug, Clone, Args)]
struct AddCli {
    #[arg(long)]
    id: String,
    #[arg(long, short = 'g')]
    global: bool,
    #[arg(long, group = "source_locator")]
    path: Option<String>,
    #[arg(long = "git", group = "source_locator")]
    git: Option<String>,
    #[arg(long, requires = "git")]
    rev: Option<String>,
    #[arg(long, requires = "git", conflicts_with = "ssh_key")]
    ssh: bool,
    #[arg(long = "ssh-key", requires = "git", conflicts_with = "ssh")]
    ssh_key: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct RemoveCli {
    #[arg(long, short = 'g')]
    global: bool,
    source_id: String,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Args)]
struct UpdateCli {
    #[arg(long, short = 'g')]
    global: bool,
    source_id: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct HelpCli {
    topic: Option<HelpTopic>,
}

impl InitCli {
    fn resolve(self) -> Result<InitRequest> {
        let scaffold_local_packages = if self.with_packages {
            true
        } else if self.without_packages {
            false
        } else if self.yes {
            false
        } else {
            ui::prompt_yes_no(
                "Scaffold local package source",
                "Do you want Ply to create a local `ply-packages/` source in this target?\n\nChoose this when you want to bake packages directly into the target root.",
                "Create local package source",
                false,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?
        };

        let ignore_config = if self.ignore_config {
            true
        } else if self.track_config {
            false
        } else if self.yes {
            true
        } else {
            ui::prompt_yes_no(
                "Ignore Ply config locally",
                "Do you want all Ply files to stay ignored in this target when it is a Git repo, including `ply.toml`, `ply.lock`, and `ply-packages/`?",
                "Keep Ply config ignored locally",
                true,
            )
            .map_err(|err| anyhow!("failed to read init option: {err}"))?
        };

        let adapters = init_adapters(&self.adapters)?;

        Ok(InitRequest {
            options: InitOptions {
                scaffold_local_packages,
                ignore_config,
                adapters,
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

fn init_adapters(selected: &[InitAdapter]) -> Result<&'static [&'static str]> {
    let has_codex = selected
        .iter()
        .any(|adapter| matches!(adapter, InitAdapter::Codex));
    let has_claude = selected
        .iter()
        .any(|adapter| matches!(adapter, InitAdapter::Claude));

    Ok(match (has_codex, has_claude) {
        (false, false) => &["codex", "claude"],
        (true, false) => &["codex"],
        (false, true) => &["claude"],
        (true, true) => &["codex", "claude"],
    })
}

impl InitPackageCli {
    fn resolve(self, project_root: &Path) -> Result<PackageInitRequest> {
        let name = self
            .name
            .ok_or_else(|| anyhow!("`ply init package` requires `--name`"))?;
        let kinds = self
            .kinds
            .iter()
            .map(|item| AssetKind::parse(item.trim()))
            .collect::<Result<Vec<_>>>()?;
        let path = self
            .path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| project_root.to_path_buf());
        Ok(PackageInitRequest {
            name,
            path,
            kinds,
            dry_run: self.dry_run,
        })
    }
}

impl From<ApplyCli> for ApplyOptions {
    fn from(value: ApplyCli) -> Self {
        Self {
            dry_run: value.dry_run,
            yes: value.yes,
        }
    }
}

impl Command {
    fn init_target(&self) -> Option<CommandTarget> {
        match self {
            Self::Apply(_) | Self::Diff => Some(CommandTarget::Project),
            Self::Doctor(target) => (!target.global).then_some(CommandTarget::Project),
            Self::List(target) => (!target.global).then_some(CommandTarget::Project),
            Self::Sources(target) => (!target.global).then_some(CommandTarget::Project),
            Self::Add(options) => Some(command_target(options.global)),
            Self::Remove(options) => Some(command_target(options.global)),
            Self::Update(options) => Some(command_target(options.global)),
            _ => None,
        }
    }
}

impl Cli {
    fn parse(args: Vec<String>) -> Result<Option<Self>> {
        match <Self as Parser>::try_parse_from(std::iter::once("ply".to_string()).chain(args)) {
            Ok(cli) => Ok(Some(cli)),
            Err(err)
                if matches!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
            {
                print!("{}", err.render());
                Ok(None)
            }
            Err(err) => Err(anyhow!(err.to_string())),
        }
    }
}

fn print_help(topic: Option<HelpTopic>) -> Result<()> {
    print!("{}", render_help(topic)?);
    Ok(())
}

fn render_help(topic: Option<HelpTopic>) -> Result<String> {
    let mut command = Cli::command();
    let target = match topic {
        None => &mut command,
        Some(topic) => command_for_topic(&mut command, topic)?,
    };

    let mut buffer = Vec::new();
    target.write_long_help(&mut buffer)?;
    String::from_utf8(buffer).map_err(|err| anyhow!("failed to render help output: {err}"))
}

fn command_for_topic<'a>(
    command: &'a mut clap::Command,
    topic: HelpTopic,
) -> Result<&'a mut clap::Command> {
    let path: &[&str] = match topic {
        HelpTopic::Init => &["init"],
        HelpTopic::InitPackage => &["init", "package"],
        HelpTopic::Apply => &["apply"],
        HelpTopic::Diff => &["diff"],
        HelpTopic::Doctor => &["doctor"],
        HelpTopic::List => &["list"],
        HelpTopic::Sources => &["sources"],
        HelpTopic::Adapters => &["adapters"],
        HelpTopic::Add => &["add"],
        HelpTopic::Remove => &["remove"],
        HelpTopic::Update => &["update"],
        HelpTopic::Clean => &["clean"],
    };

    let mut current = command;
    for segment in path {
        current = current
            .find_subcommand_mut(segment)
            .ok_or_else(|| anyhow!("unknown help topic `{segment}`"))?;
    }
    Ok(current)
}

fn command_target(global: bool) -> CommandTarget {
    if global {
        CommandTarget::Global
    } else {
        CommandTarget::Project
    }
}

fn resolve_target_root(project_root: &Path, target: CommandTarget) -> Result<std::path::PathBuf> {
    match target {
        CommandTarget::Project => Ok(project_root.to_path_buf()),
        CommandTarget::Global => config::global_root(),
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
            Command::Apply(ApplyCli {
                dry_run: false,
                yes: false,
            }),
            Command::Diff,
            Command::Doctor(TargetCli { global: false }),
            Command::List(TargetCli { global: false }),
            Command::Sources(TargetCli { global: false }),
            Command::Add(AddCli {
                id: "local".to_string(),
                global: false,
                path: Some("./ply-packages/example-review".to_string()),
                git: None,
                rev: None,
                ssh: false,
                ssh_key: None,
            }),
            Command::Remove(RemoveCli {
                global: false,
                source_id: "local".to_string(),
                force: false,
            }),
            Command::Update(UpdateCli {
                global: false,
                source_id: None,
            }),
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
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
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
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Clean(options) => {
                assert!(options.global);
                assert!(options.dry_run);
            }
            other => panic!("expected clean command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_init_adapters_flag() -> Result<()> {
        let cli = Cli::parse(vec![
            "init".to_string(),
            "--adapters".to_string(),
            "claude".to_string(),
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Init(options) => {
                assert_eq!(options.options.adapters, vec![InitAdapter::Claude]);
            }
            other => panic!("expected init command, got {other:?}"),
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
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Init(InitCommand {
                command: Some(InitSubcommand::Package(options)),
                ..
            }) => {
                assert_eq!(options.name.as_deref(), Some("review-tools"));
                assert_eq!(options.path.as_deref(), Some("./packages/review-tools"));
                assert_eq!(
                    options.kinds,
                    vec!["skills".to_string(), "commands".to_string()]
                );
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
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Init(InitCommand {
                command: Some(InitSubcommand::Package(options)),
                ..
            }) => {
                assert_eq!(options.kinds, vec!["agents".to_string()]);
            }
            other => panic!("expected init package command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn init_yes_defaults_to_ignored_config() -> Result<()> {
        let request = InitCli {
            with_packages: false,
            without_packages: true,
            ignore_config: false,
            track_config: false,
            adapters: Vec::new(),
            yes: true,
            global: false,
            dry_run: false,
        }
        .resolve()?;

        assert!(request.options.ignore_config);
        Ok(())
    }

    #[test]
    fn init_adapters_can_be_scoped_to_claude() -> Result<()> {
        let request = InitCli {
            with_packages: false,
            without_packages: true,
            ignore_config: false,
            track_config: false,
            adapters: vec![InitAdapter::Claude],
            yes: true,
            global: false,
            dry_run: false,
        }
        .resolve()?;

        assert_eq!(request.options.adapters, &["claude"]);
        Ok(())
    }

    #[test]
    fn parse_add_git_source_flags() -> Result<()> {
        let cli = Cli::parse(vec![
            "add".to_string(),
            "--id".to_string(),
            "team".to_string(),
            "--git".to_string(),
            "owner/repo".to_string(),
            "--rev".to_string(),
            "main".to_string(),
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Add(options) => {
                assert_eq!(options.id, "team");
                assert!(!options.global);
                assert_eq!(options.git.as_deref(), Some("owner/repo"));
                assert_eq!(options.rev.as_deref(), Some("main"));
                assert!(!options.ssh);
                assert_eq!(options.ssh_key, None);
            }
            other => panic!("expected add command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_global_update_flag() -> Result<()> {
        let cli = Cli::parse(vec![
            "update".to_string(),
            "--global".to_string(),
            "team".to_string(),
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Update(options) => {
                assert!(options.global);
                assert_eq!(options.source_id.as_deref(), Some("team"));
            }
            other => panic!("expected update command, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn parse_add_git_source_ssh_key_flag() -> Result<()> {
        let cli = Cli::parse(vec![
            "add".to_string(),
            "--id".to_string(),
            "team".to_string(),
            "--git".to_string(),
            "owner/repo".to_string(),
            "--ssh-key".to_string(),
            "~/.ssh/id_team".to_string(),
        ])?
        .expect("cli should parse");
        match cli.command.expect("command should be present") {
            Command::Add(options) => {
                assert_eq!(options.ssh_key.as_deref(), Some("~/.ssh/id_team"));
                assert!(!options.ssh);
            }
            other => panic!("expected add command, got {other:?}"),
        }
        Ok(())
    }
}
