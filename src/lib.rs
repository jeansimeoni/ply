mod adapters;
mod config;
mod git;
mod ops;

use anyhow::{Result, anyhow};
use std::env;

pub fn run() -> Result<()> {
    let cli = Cli::parse(env::args().skip(1).collect())?;
    let project_root = env::current_dir()?;
    match cli.command {
        Command::Init => {
            ops::init_project(&project_root)?;
            println!("initialized ply project in {}", project_root.display());
        }
        Command::Apply => {
            let summary = ops::apply(&project_root)?;
            println!("{summary}");
        }
        Command::Diff => {
            let summary = ops::diff(&project_root)?;
            println!("{summary}");
        }
        Command::Doctor => {
            let summary = ops::doctor(&project_root)?;
            println!("{summary}");
        }
        Command::List => {
            let summary = ops::list_packages(&project_root)?;
            println!("{summary}");
        }
        Command::Sources => {
            let summary = ops::list_sources(&project_root)?;
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

#[derive(Debug)]
struct Cli {
    command: Command,
}

#[derive(Debug)]
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
