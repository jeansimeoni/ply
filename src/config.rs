use anyhow::{Context, Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    #[serde(default = "default_use_global")]
    pub use_global: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub kind: String,
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub rev: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub sources: Vec<LocalSourceConfig>,
    #[serde(default)]
    pub overlays: Vec<OverlayEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalSourceConfig {
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
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
pub struct SshConfigFile {
    #[serde(default)]
    pub sources: BTreeMap<String, SshSourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SshSourceConfig {
    #[serde(default)]
    pub use_ssh: bool,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    #[serde(default)]
    pub ssh_key_env: Option<String>,
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
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    pub resolved: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub ignore_config: bool,
    #[serde(default)]
    pub owned_paths: Vec<OwnedPath>,
}

#[derive(Debug, Clone, Copy)]
pub struct InitOptions {
    pub scaffold_local_packages: bool,
    pub ignore_config: bool,
    pub adapters: &'static [&'static str],
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
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
}

fn default_schema_version() -> u32 {
    1
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
            use_global: default_use_global(),
        }
    }
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

pub fn load_local_manifest_if_present(project_root: &Path) -> Result<Option<LocalManifest>> {
    let path = project_root.join("ply.local.toml");
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: LocalManifest =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_local_manifest(&manifest)?;
    Ok(Some(manifest))
}

pub fn load_ssh_config_if_present(project_root: &Path) -> Result<Option<SshConfigFile>> {
    let path = project_root.join("ply.ssh.toml");
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let config: SshConfigFile =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_ssh_config(&config)?;
    Ok(Some(config))
}

pub fn load_ssh_config_for_edit(project_root: &Path) -> Result<SshConfigFile> {
    Ok(load_ssh_config_if_present(project_root)?.unwrap_or_default())
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

pub fn load_manifest_for_edit(project_root: &Path) -> Result<Manifest> {
    load_manifest(project_root)
}

pub fn load_local_overlays(project_root: &Path) -> Result<LocalOverlayConfig> {
    let mut merged = LocalOverlayConfig::default();

    if let Some(local_manifest) = load_local_manifest_if_present(project_root)? {
        merged.overlays.extend(local_manifest.overlays);
    }

    let legacy_path = project_root.join(".ply").join("local.yml");
    if legacy_path.exists() {
        let content = fs::read_to_string(&legacy_path)
            .with_context(|| format!("failed to read {}", legacy_path.display()))?;
        let overlays: LocalOverlayConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse {}", legacy_path.display()))?;
        merged.overlays.extend(overlays.overlays);
    }

    validate_local_overlays(&merged)?;
    Ok(deduplicate_overlays(merged))
}

pub fn load_state(project_root: &Path) -> Result<State> {
    let path = project_root.join(".ply").join("state.json");
    if !path.exists() {
        return Ok(State {
            schema_version: 1,
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

pub fn load_lockfile_if_present(project_root: &Path) -> Result<Option<Lockfile>> {
    let path = project_root.join("ply.lock");
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let lockfile: Lockfile =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(lockfile))
}

pub fn write_manifest(project_root: &Path, manifest: &Manifest) -> Result<()> {
    validate_manifest(manifest)?;
    let path = project_root.join("ply.toml");
    let mut sorted = manifest.clone();
    sorted.sources.sort_by(|a, b| a.id.cmp(&b.id));
    let content = toml::to_string_pretty(&sorted)?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_ssh_config(project_root: &Path, config: &SshConfigFile) -> Result<()> {
    validate_ssh_config(config)?;
    let path = project_root.join("ply.ssh.toml");
    if config.sources.is_empty() {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        return Ok(());
    }

    let content = toml::to_string_pretty(config)?;
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
    let mut manifest = Manifest {
        schema_version: 1,
        install: InstallConfig::default(),
        adapters: options
            .adapters
            .iter()
            .map(|adapter| adapter.to_string())
            .collect(),
        sources: Vec::new(),
    };
    if options.scaffold_local_packages {
        manifest.sources.push(SourceConfig {
            id: "local".to_string(),
            kind: "path".to_string(),
            path: Some("./ply-packages/example-review".to_string()),
            repo: None,
            url: None,
            rev: None,
        });
    }
    write_manifest(project_root, &manifest)
}

pub fn write_default_local_manifest(project_root: &Path) -> Result<()> {
    let path = project_root.join("ply.local.toml");
    if path.exists() {
        return Ok(());
    }

    let template = if let Some(legacy) = load_legacy_local_overlay_config(project_root)? {
        render_local_manifest_template(&legacy.overlays)
    } else {
        render_local_manifest_template(&default_overlay_entries())
    };
    fs::write(&path, template).with_context(|| format!("failed to write {}", path.display()))
}

pub fn write_default_package_fixture(project_root: &Path) -> Result<()> {
    let package_dir = project_root.join("ply-packages").join("example-review");
    let skill = package_dir.join("skills").join("review-diff");
    fs::create_dir_all(&skill)?;
    let pkg = package_dir.join("ply-package.toml");
    if !pkg.exists() {
        fs::write(
            &pkg,
            "name = \"review-diff\"\ndescription = \"Review-diff skill\"\n",
        )?;
    }
    let readme = skill.join("SKILL.md");
    if !readme.exists() {
        fs::write(
            &readme,
            "# review-diff\n\nReview a diff with a bug-first mindset.\n",
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
    write_package_manifest_contents(
        package_root,
        &PackageManifest {
            name: name.to_string(),
            version: None,
            description: None,
            license: None,
            targets: Vec::new(),
        },
    )
}

pub fn load_package_manifest(package_root: &Path) -> Result<PackageManifest> {
    let manifest = load_package_manifest_for_edit(package_root)?;
    validate_package_manifest(&manifest)?;
    Ok(manifest)
}

pub fn load_package_manifest_for_edit(package_root: &Path) -> Result<PackageManifest> {
    let path = package_root.join("ply-package.toml");
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: PackageManifest =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(manifest)
}

pub fn write_package_manifest_contents(
    package_root: &Path,
    manifest: &PackageManifest,
) -> Result<()> {
    validate_package_manifest(manifest)?;
    fs::create_dir_all(package_root)?;
    let path = package_root.join("ply-package.toml");
    let content = toml::to_string_pretty(manifest)?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

pub fn validate_package_manifest(manifest: &PackageManifest) -> Result<()> {
    if manifest.name.trim().is_empty() {
        return Err(anyhow!("package name cannot be empty"));
    }
    if let Some(version) = manifest.version.as_deref() {
        Version::parse(version)
            .with_context(|| format!("package version `{version}` is not valid semver"))?;
    }
    for target in &manifest.targets {
        AdapterKind::parse(target)?;
    }
    Ok(())
}

pub fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.schema_version != 1 {
        return Err(anyhow!(
            "unsupported schema_version `{}`; only version 1 is supported",
            manifest.schema_version
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
                if source.repo.is_none() && source.url.is_none() {
                    return Err(anyhow!("git source `{}` is missing `repo`", source.id));
                }
                if source.repo.is_some() && source.url.is_some() {
                    return Err(anyhow!(
                        "git source `{}` cannot define both `repo` and legacy `url`",
                        source.id
                    ));
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
        if matches!(
            adapter.exposure_mode(kind),
            ExposureMode::GeneratedComposite
        ) && !kind.is_directory_based()
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

fn validate_local_manifest(manifest: &LocalManifest) -> Result<()> {
    if manifest.schema_version != 1 {
        return Err(anyhow!(
            "unsupported schema_version `{}`; only version 1 is supported",
            manifest.schema_version
        ));
    }
    for source in &manifest.sources {
        if source.id.trim().is_empty() {
            return Err(anyhow!("local source id cannot be empty"));
        }
        if source.repo.is_some() && source.url.is_some() {
            return Err(anyhow!(
                "local source `{}` cannot define both `repo` and legacy `url`",
                source.id
            ));
        }
    }
    validate_local_overlays(&LocalOverlayConfig {
        overlays: manifest.overlays.clone(),
    })?;
    Ok(())
}

fn load_legacy_local_overlay_config(project_root: &Path) -> Result<Option<LocalOverlayConfig>> {
    let path = project_root.join(".ply").join("local.yml");
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let overlays: LocalOverlayConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    validate_local_overlays(&overlays)?;
    Ok(Some(overlays))
}

fn default_overlay_entries() -> Vec<OverlayEntry> {
    vec![
        OverlayEntry {
            adapter: "codex".to_string(),
            kind: "skills".to_string(),
            path: ".ply/overlays/codex/skills".to_string(),
        },
        OverlayEntry {
            adapter: "claude".to_string(),
            kind: "skills".to_string(),
            path: ".ply/overlays/claude/skills".to_string(),
        },
    ]
}

fn render_local_manifest_template(overlays: &[OverlayEntry]) -> String {
    let mut template = String::from("schema_version = 1\n");
    for overlay in overlays {
        template.push_str("\n[[overlays]]\n");
        template.push_str(&format!("adapter = \"{}\"\n", overlay.adapter));
        template.push_str(&format!("kind = \"{}\"\n", overlay.kind));
        template.push_str(&format!("path = \"{}\"\n", overlay.path));
    }
    template
}

fn deduplicate_overlays(config: LocalOverlayConfig) -> LocalOverlayConfig {
    let mut seen = BTreeSet::new();
    let mut overlays = Vec::new();
    for overlay in config.overlays {
        let key = format!("{}::{}::{}", overlay.adapter, overlay.kind, overlay.path);
        if seen.insert(key) {
            overlays.push(overlay);
        }
    }
    LocalOverlayConfig { overlays }
}

fn validate_ssh_config(config: &SshConfigFile) -> Result<()> {
    for (source_id, source) in &config.sources {
        if source.ssh_key_path.is_some() && source.ssh_key_env.is_some() {
            return Err(anyhow!(
                "ssh config for source `{source_id}` cannot define both `ssh_key_path` and `ssh_key_env`"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn accept_manifest_with_single_source_package_root() -> Result<()> {
        let manifest = Manifest {
            schema_version: 1,
            install: InstallConfig::default(),
            adapters: vec!["codex".to_string()],
            sources: vec![SourceConfig {
                id: "local".to_string(),
                kind: "path".to_string(),
                path: Some("./packages/example".to_string()),
                repo: None,
                url: None,
                rev: None,
            }],
        };

        validate_manifest(&manifest)?;
        Ok(())
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

    #[test]
    fn local_manifest_accepts_overlays() -> Result<()> {
        let manifest = LocalManifest {
            schema_version: 1,
            sources: Vec::new(),
            overlays: vec![OverlayEntry {
                adapter: "claude".to_string(),
                kind: "skills".to_string(),
                path: ".ply/overlays/claude/skills".to_string(),
            }],
        };

        validate_local_manifest(&manifest)?;
        Ok(())
    }

    #[test]
    fn load_local_overlays_merges_toml_and_legacy_yaml() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join(".ply"))?;
        fs::write(
            temp.path().join("ply.local.toml"),
            r#"schema_version = 1

[[overlays]]
adapter = "codex"
kind = "skills"
path = ".ply/overlays/codex/skills"
"#,
        )?;
        fs::write(
            temp.path().join(".ply").join("local.yml"),
            r#"overlays:
  - adapter: claude
    kind: skills
    path: .ply/overlays/claude/skills
"#,
        )?;

        let overlays = load_local_overlays(temp.path())?;
        assert_eq!(overlays.overlays.len(), 2);
        assert!(
            overlays
                .overlays
                .iter()
                .any(|overlay| overlay.adapter == "codex")
        );
        assert!(
            overlays
                .overlays
                .iter()
                .any(|overlay| overlay.adapter == "claude")
        );
        Ok(())
    }

    #[test]
    fn write_default_local_manifest_migrates_legacy_overlay_file() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join(".ply"))?;
        fs::write(
            temp.path().join(".ply").join("local.yml"),
            r#"overlays:
  - adapter: claude
    kind: skills
    path: .ply/overlays/claude/skills
"#,
        )?;

        write_default_local_manifest(temp.path())?;

        let written = fs::read_to_string(temp.path().join("ply.local.toml"))?;
        assert!(written.contains("[[overlays]]"));
        assert!(written.contains("adapter = \"claude\""));
        assert!(written.contains("path = \".ply/overlays/claude/skills\""));
        Ok(())
    }

    #[test]
    fn write_default_manifest_respects_selected_adapters() -> Result<()> {
        let temp = TempDir::new()?;

        write_default_manifest(
            temp.path(),
            InitOptions {
                scaffold_local_packages: false,
                ignore_config: false,
                adapters: &["claude"],
            },
        )?;

        let written = fs::read_to_string(temp.path().join("ply.toml"))?;
        assert!(written.contains("adapters = [\"claude\"]"));
        assert!(!written.contains("\"codex\""));
        Ok(())
    }

    #[test]
    fn reject_manifest_with_legacy_install_mode_field() {
        let err = toml::from_str::<Manifest>(
            r#"
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field `mode`"));
    }

    #[test]
    fn reject_state_with_legacy_install_mode_field() {
        let err = serde_json::from_str::<State>(
            r#"{
  "schema_version": 1,
  "install_mode": "copy",
  "ignore_config": false,
  "owned_paths": []
}"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field `install_mode`"));
    }

    #[test]
    fn reject_git_source_with_both_repo_and_url() {
        let manifest = Manifest {
            schema_version: 1,
            install: InstallConfig::default(),
            adapters: vec!["codex".to_string()],
            sources: vec![SourceConfig {
                id: "team".to_string(),
                kind: "git".to_string(),
                path: None,
                repo: Some("owner/repo".to_string()),
                url: Some("https://example.com/repo.git".to_string()),
                rev: None,
            }],
        };

        let err = validate_manifest(&manifest).unwrap_err();
        assert!(err.to_string().contains("both `repo` and legacy `url`"));
    }

    #[test]
    fn reject_ssh_config_with_two_key_sources() {
        let config = SshConfigFile {
            sources: BTreeMap::from([(
                "team".to_string(),
                SshSourceConfig {
                    use_ssh: true,
                    ssh_key_path: Some("~/.ssh/id_test".to_string()),
                    ssh_key_env: Some("PLY_KEY".to_string()),
                },
            )]),
        };

        let err = validate_ssh_config(&config).unwrap_err();
        assert!(
            err.to_string()
                .contains("both `ssh_key_path` and `ssh_key_env`")
        );
    }

    #[test]
    fn reject_invalid_package_semver() {
        let manifest = PackageManifest {
            name: "review-tools".to_string(),
            version: Some("not-semver".to_string()),
            description: None,
            license: None,
            targets: Vec::new(),
        };

        let err = validate_package_manifest(&manifest).unwrap_err();
        assert!(err.to_string().contains("not valid semver"));
    }

    #[test]
    fn reject_unknown_package_target() {
        let manifest = PackageManifest {
            name: "review-tools".to_string(),
            version: None,
            description: None,
            license: None,
            targets: vec!["cursor".to_string()],
        };

        let err = validate_package_manifest(&manifest).unwrap_err();
        assert!(err.to_string().contains("unsupported adapter `cursor`"));
    }
}
