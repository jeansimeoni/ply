use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::adapters::{AdapterKind, AssetKind, ExposureMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub install: InstallConfig,
    #[serde(default = "default_adapters")]
    pub adapters: Vec<String>,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub packages: Vec<PackageSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    #[serde(default = "default_install_mode")]
    pub mode: String,
    #[serde(default = "default_use_global")]
    pub use_global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub kind: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub rev: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSelection {
    pub source: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalOverlayConfig {
    #[serde(default)]
    pub overlays: Vec<OverlayEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayEntry {
    pub adapter: String,
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub sources: Vec<LockedSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedSource {
    pub id: String,
    pub kind: String,
    pub resolved: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub install_mode: String,
    #[serde(default)]
    pub ignore_config: bool,
    #[serde(default)]
    pub owned_paths: Vec<OwnedPath>,
}

#[derive(Debug, Clone, Copy)]
pub struct InitOptions {
    pub scaffold_local_packages: bool,
    pub ignore_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedPath {
    pub adapter: String,
    pub kind: String,
    #[serde(default)]
    pub exposure_mode: String,
    pub relative_name: String,
    pub generated_path: String,
    pub exposed_path: String,
    #[serde(default)]
    pub generated_digest: String,
    #[serde(default)]
    pub exposed_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

fn default_install_mode() -> String {
    "copy".to_string()
}

fn default_use_global() -> bool {
    true
}

fn default_adapters() -> Vec<String> {
    vec!["codex".to_string(), "claude".to_string()]
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            mode: default_install_mode(),
            use_global: default_use_global(),
        }
    }
}

pub fn ensure_initialized(project_root: &Path) -> Result<()> {
    ensure_initialized_with_hint(project_root, "ply init")
}

pub fn ensure_initialized_with_hint(project_root: &Path, hint: &str) -> Result<()> {
    let path = project_root.join("ply.toml");
    if path.exists() {
        return Ok(());
    }

    Err(anyhow!(
        "ply is not initialized in {}; run `{hint}` to scaffold ply.toml and local state files",
        project_root.display(),
    ))
}

pub fn global_root() -> Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(std::path::PathBuf::from(home).join(".config").join("ply"))
}

pub fn load_manifest_if_present(project_root: &Path) -> Result<Option<Manifest>> {
    let path = project_root.join("ply.toml");
    if !path.exists() {
        return Ok(None);
    }
    load_manifest(project_root).map(Some)
}

pub fn load_manifest(project_root: &Path) -> Result<Manifest> {
    let path = project_root.join("ply.toml");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn load_local_overlays(project_root: &Path) -> Result<LocalOverlayConfig> {
    let path = project_root.join(".ply").join("local.yml");
    if !path.exists() {
        return Ok(LocalOverlayConfig::default());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let overlays: LocalOverlayConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    validate_local_overlays(&overlays)?;
    Ok(overlays)
}

pub fn load_state(project_root: &Path) -> Result<State> {
    let path = project_root.join(".ply").join("state.json");
    if !path.exists() {
        return Ok(State {
            schema_version: 1,
            install_mode: default_install_mode(),
            ignore_config: false,
            owned_paths: Vec::new(),
        });
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let state = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(state)
}

pub fn write_lockfile(project_root: &Path, lockfile: &Lockfile) -> Result<()> {
    let path = project_root.join("ply.lock");
    let mut sorted = lockfile.clone();
    sorted.sources.sort_by(|a, b| a.id.cmp(&b.id));
    let content = toml::to_string_pretty(&sorted)?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_state(project_root: &Path, state: &State) -> Result<()> {
    let ply_dir = project_root.join(".ply");
    fs::create_dir_all(&ply_dir)?;
    let path = ply_dir.join("state.json");
    let content = serde_json::to_string_pretty(state)?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_default_manifest(project_root: &Path, options: InitOptions) -> Result<()> {
    let path = project_root.join("ply.toml");
    if path.exists() {
        return Ok(());
    }
    let template = if options.scaffold_local_packages {
        r#"schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true

[[sources]]
id = "local"
kind = "path"
path = "./ply-packages"

[[packages]]
source = "local"
path = "example-review"
"#
    } else {
        r#"schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true
"#
    };
    fs::write(&path, template).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_default_local_overlay(project_root: &Path) -> Result<()> {
    let ply_dir = project_root.join(".ply");
    fs::create_dir_all(&ply_dir)?;
    let path = ply_dir.join("local.yml");
    if path.exists() {
        return Ok(());
    }
    let template = r#"overlays:
  - adapter: codex
    kind: skills
    path: .ply/overlays/codex/skills
  - adapter: claude
    kind: skills
    path: .ply/overlays/claude/skills
"#;
    fs::write(&path, template).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_default_package_fixture(project_root: &Path) -> Result<()> {
    let package_dir = project_root.join("ply-packages").join("example-review");
    let codex_skill = package_dir
        .join("codex")
        .join("skills")
        .join("ply-review-diff");
    let claude_skill = package_dir
        .join("claude")
        .join("skills")
        .join("ply-review-diff");
    fs::create_dir_all(&codex_skill)?;
    fs::create_dir_all(&claude_skill)?;
    let pkg = package_dir.join("ply-package.toml");
    if !pkg.exists() {
        fs::write(
            &pkg,
            "name = \"ply-review-diff\"\ndescription = \"Review-diff skill\"\n",
        )?;
    }
    let codex_readme = codex_skill.join("SKILL.md");
    if !codex_readme.exists() {
        fs::write(
            &codex_readme,
            "# ply-review-diff\n\nReview a diff with a bug-first mindset.\n",
        )?;
    }
    let claude_readme = claude_skill.join("SKILL.md");
    if !claude_readme.exists() {
        fs::write(
            &claude_readme,
            "# ply-review-diff\n\nReview a diff with a bug-first mindset.\n",
        )?;
    }
    Ok(())
}

pub fn write_package_manifest(package_root: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(package_root)?;
    let path = package_root.join("ply-package.toml");
    if path.exists() {
        return Ok(());
    }
    let content = format!("name = \"{name}\"\n");
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

pub fn load_package_manifest(package_root: &Path) -> Result<PackageManifest> {
    let path = package_root.join("ply-package.toml");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(manifest)
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.schema_version != 1 {
        return Err(anyhow!(
            "unsupported schema_version `{}`; only version 1 is supported",
            manifest.schema_version
        ));
    }
    if manifest.install.mode != "copy" {
        return Err(anyhow!(
            "unsupported install mode `{}`; only `copy` is implemented",
            manifest.install.mode
        ));
    }
    let mut source_ids = BTreeSet::new();
    for source in &manifest.sources {
        if !source_ids.insert(source.id.clone()) {
            return Err(anyhow!("duplicate source id `{}`", source.id));
        }
        match source.kind.as_str() {
            "path" => {
                if source.path.is_none() {
                    return Err(anyhow!("path source `{}` is missing `path`", source.id));
                }
            }
            "git" => {
                if source.url.is_none() {
                    return Err(anyhow!("git source `{}` is missing `url`", source.id));
                }
            }
            other => {
                return Err(anyhow!(
                    "unsupported source kind `{other}` for source `{}`",
                    source.id
                ));
            }
        }
    }

    let supported_adapters = ["codex", "claude"];
    for adapter in &manifest.adapters {
        if !supported_adapters.contains(&adapter.as_str()) {
            return Err(anyhow!("unsupported adapter `{adapter}`"));
        }
    }

    for package in &manifest.packages {
        if !source_ids.contains(&package.source) {
            return Err(anyhow!(
                "package `{}` references unknown source `{}`",
                package.path,
                package.source
            ));
        }
        if package.path.trim().is_empty() {
            return Err(anyhow!("package path cannot be empty"));
        }
    }

    let mut unique_packages = BTreeSet::new();
    for package in &manifest.packages {
        let key = format!("{}::{}", package.source, package.path);
        if !unique_packages.insert(key) {
            return Err(anyhow!(
                "duplicate package selection `{}` from source `{}`",
                package.path,
                package.source
            ));
        }
    }

    Ok(())
}

fn validate_local_overlays(overlays: &LocalOverlayConfig) -> Result<()> {
    for overlay in &overlays.overlays {
        let adapter = AdapterKind::parse(&overlay.adapter)?;
        let kind = AssetKind::parse(&overlay.kind)?;
        if !adapter.supports(kind) {
            return Err(anyhow!(
                "adapter `{}` does not support asset kind `{}`",
                overlay.adapter,
                overlay.kind
            ));
        }
        if matches!(adapter.exposure_mode(kind), ExposureMode::GeneratedComposite)
            && !kind.is_directory_based()
            && !overlay.path.ends_with(".md")
        {
            return Err(anyhow!(
                "overlay `{}` for `{}` `{}` must point to a markdown file",
                overlay.path,
                overlay.adapter,
                overlay.kind
            ));
        }
        if overlay.path.trim().is_empty() {
            return Err(anyhow!(
                "overlay path cannot be empty for `{}` `{}`",
                overlay.adapter,
                overlay.kind
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_duplicate_package_selection() {
        let manifest = Manifest {
            schema_version: 1,
            install: InstallConfig::default(),
            adapters: vec!["codex".to_string()],
            sources: vec![SourceConfig {
                id: "local".to_string(),
                kind: "path".to_string(),
                path: Some("./packages".to_string()),
                url: None,
                rev: None,
            }],
            packages: vec![
                PackageSelection {
                    source: "local".to_string(),
                    path: "example".to_string(),
                },
                PackageSelection {
                    source: "local".to_string(),
                    path: "example".to_string(),
                },
            ],
        };

        let err = validate_manifest(&manifest).unwrap_err();
        assert!(err.to_string().contains("duplicate package selection"));
    }

    #[test]
    fn reject_overlay_for_unknown_asset_kind() {
        let overlays = LocalOverlayConfig {
            overlays: vec![OverlayEntry {
                adapter: "codex".to_string(),
                kind: "settings".to_string(),
                path: ".ply/overlays/codex/settings".to_string(),
            }],
        };

        let err = validate_local_overlays(&overlays).unwrap_err();
        assert!(err.to_string().contains("unsupported asset kind"));
    }
}
