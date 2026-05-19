use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Commands,
    Skills,
    Agents,
    LocalInstructions,
    Rules,
    Hooks,
    OutputStyles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureMode {
    Direct,
    InjectBlock,
    GeneratedComposite,
}

impl ExposureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::InjectBlock => "inject-block",
            Self::GeneratedComposite => "generated-composite",
        }
    }
}

impl AdapterKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            other => Err(anyhow!("unsupported adapter `{other}`")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub fn supports(self, kind: AssetKind) -> bool {
        match (self, kind) {
            (Self::Codex, AssetKind::Commands)
            | (Self::Codex, AssetKind::Skills)
            | (Self::Codex, AssetKind::LocalInstructions)
            | (Self::Codex, AssetKind::Rules)
            | (Self::Codex, AssetKind::Hooks)
            | (Self::Codex, AssetKind::OutputStyles)
            | (Self::Claude, AssetKind::Commands)
            | (Self::Claude, AssetKind::Skills)
            | (Self::Claude, AssetKind::Agents)
            | (Self::Claude, AssetKind::LocalInstructions)
            | (Self::Claude, AssetKind::Rules)
            | (Self::Claude, AssetKind::Hooks)
            | (Self::Claude, AssetKind::OutputStyles) => true,
            _ => false,
        }
    }

    pub fn exposure_mode(self, kind: AssetKind) -> ExposureMode {
        match (self, kind) {
            (Self::Claude, AssetKind::LocalInstructions) => ExposureMode::InjectBlock,
            (Self::Codex, AssetKind::LocalInstructions)
            | (Self::Codex, AssetKind::OutputStyles) => ExposureMode::GeneratedComposite,
            _ => ExposureMode::Direct,
        }
    }

    pub fn direct_asset_root(self, project_root: &Path, kind: AssetKind) -> Option<PathBuf> {
        match (self, kind) {
            (Self::Codex, AssetKind::Commands) => Some(project_root.join(".agents").join("commands")),
            (Self::Codex, AssetKind::Skills) => Some(project_root.join(".agents").join("skills")),
            (Self::Codex, AssetKind::Rules) => Some(project_root.join(".codex").join("rules")),
            (Self::Codex, AssetKind::Hooks) => Some(project_root.join(".codex").join("hooks")),
            (Self::Claude, AssetKind::Commands) => Some(project_root.join(".claude").join("commands")),
            (Self::Claude, AssetKind::Skills) => Some(project_root.join(".claude").join("skills")),
            (Self::Claude, AssetKind::Agents) => Some(project_root.join(".claude").join("agents")),
            (Self::Claude, AssetKind::Rules) => Some(project_root.join(".claude").join("rules")),
            (Self::Claude, AssetKind::Hooks) => Some(project_root.join(".claude").join("hooks")),
            (Self::Claude, AssetKind::OutputStyles) => {
                Some(project_root.join(".claude").join("output-styles"))
            }
            _ => None,
        }
    }

    pub fn managed_file_path(self, project_root: &Path, kind: AssetKind) -> Option<PathBuf> {
        match (self, kind) {
            (Self::Claude, AssetKind::LocalInstructions) => {
                Some(project_root.join("CLAUDE.local.md"))
            }
            (Self::Codex, AssetKind::LocalInstructions)
            | (Self::Codex, AssetKind::OutputStyles) => Some(project_root.join("AGENTS.override.md")),
            _ => None,
        }
    }

}

impl AssetKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "commands" => Ok(Self::Commands),
            "skills" => Ok(Self::Skills),
            "agents" => Ok(Self::Agents),
            "local-instructions" => Ok(Self::LocalInstructions),
            "rules" => Ok(Self::Rules),
            "hooks" => Ok(Self::Hooks),
            "output-styles" => Ok(Self::OutputStyles),
            other => Err(anyhow!("unsupported asset kind `{other}`")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Skills => "skills",
            Self::Agents => "agents",
            Self::LocalInstructions => "local-instructions",
            Self::Rules => "rules",
            Self::Hooks => "hooks",
            Self::OutputStyles => "output-styles",
        }
    }

    pub fn is_directory_based(self) -> bool {
        !matches!(self, Self::LocalInstructions)
    }

    pub fn requires_ply_prefix(self) -> bool {
        !matches!(self, Self::LocalInstructions)
    }
}

pub fn adapter_summary() -> String {
    [
        ui::list_item("codex: commands -> .agents/commands"),
        ui::list_item("codex: skills   -> .agents/skills"),
        ui::list_item("codex: local-instructions -> AGENTS.override.md"),
        ui::list_item("codex: rules -> .codex/rules"),
        ui::list_item("codex: hooks -> .codex/hooks + .codex/hooks.json"),
        ui::list_item("codex: output-styles -> AGENTS.override.md"),
        ui::list_item("claude: commands -> .claude/commands"),
        ui::list_item("claude: skills   -> .claude/skills"),
        ui::list_item("claude: agents   -> .claude/agents"),
        ui::list_item("claude: local-instructions -> CLAUDE.local.md"),
        ui::list_item("claude: rules -> .claude/rules"),
        ui::list_item("claude: hooks -> .claude/hooks"),
        ui::list_item("claude: output-styles -> .claude/output-styles"),
    ]
    .join("\n")
}
