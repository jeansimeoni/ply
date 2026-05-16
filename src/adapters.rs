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

    pub fn asset_root(self, project_root: &Path, kind: AssetKind) -> PathBuf {
        match (self, kind) {
            (Self::Codex, AssetKind::Commands) => project_root.join(".agents").join("commands"),
            (Self::Codex, AssetKind::Skills) => project_root.join(".agents").join("skills"),
            (Self::Claude, AssetKind::Commands) => project_root.join(".claude").join("commands"),
            (Self::Claude, AssetKind::Skills) => project_root.join(".claude").join("skills"),
        }
    }
}

impl AssetKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "commands" => Ok(Self::Commands),
            "skills" => Ok(Self::Skills),
            other => Err(anyhow!("unsupported asset kind `{other}`")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Skills => "skills",
        }
    }
}

pub fn adapter_summary() -> String {
    [
        ui::list_item("codex: commands -> .agents/commands"),
        ui::list_item("codex: skills   -> .agents/skills"),
        ui::list_item("claude: commands -> .claude/commands"),
        ui::list_item("claude: skills   -> .claude/skills"),
    ]
    .join("\n")
}
