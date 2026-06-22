use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::adapters::{AdapterKind, AssetKind, ExposureMode};
use crate::config::{
    self, InitOptions, LocalManifest, LocalOverlayConfig, LocalSourceConfig, LockedSource,
    Lockfile, Manifest, OverlayEntry, OwnedPath, PackageManifest, SourceConfig, SshConfigFile,
    SshSourceConfig, State, load_local_manifest_if_present, load_local_overlays,
    load_lockfile_if_present, load_manifest, load_manifest_for_edit, load_manifest_if_present,
    load_package_manifest, load_package_manifest_for_edit, load_ssh_config_for_edit,
    load_ssh_config_if_present, load_state,
};
use crate::git;
use crate::prompt_resources::{
    is_prompt_resource, parse_prompt_resource, primary_markdown_name, prompt_logical_name,
    render_claude_markdown, render_codex_agent, render_codex_command_metadata,
    render_codex_prompt_preamble, render_codex_skill_markdown, render_codex_skill_sidecar,
};
use crate::ui::{self, Tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Global,
    Project,
}

impl LayerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone)]
struct LayerConfig {
    kind: LayerKind,
    root: PathBuf,
    manifest: Manifest,
    ssh_config: SshConfigFile,
    overlays: LocalOverlayConfig,
}

#[derive(Debug, Clone)]
struct MergedSource {
    config: SourceConfig,
    ssh_config: Option<SshSourceConfig>,
    root: PathBuf,
    layer: LayerKind,
}

#[derive(Debug, Clone)]
struct MergedOverlay {
    entry: OverlayEntry,
    root: PathBuf,
    layer: LayerKind,
}

#[derive(Debug, Clone)]
struct ComposedConfig {
    adapters: Vec<String>,
    sources: Vec<MergedSource>,
    overlays: Vec<MergedOverlay>,
}

#[derive(Debug, Clone)]
struct ResolvedSource {
    id: String,
    kind: String,
    root: PathBuf,
    resolved: String,
    locator_path: Option<String>,
    locator_repo: Option<String>,
    layer: LayerKind,
}

#[derive(Debug, Clone)]
struct ResolvedPackage {
    source_id: String,
    source_layer: LayerKind,
    root: PathBuf,
    manifest: PackageManifest,
}

#[derive(Debug, Clone)]
struct PlannedFile {
    adapter: AdapterKind,
    kind: AssetKind,
    exposure_mode: ExposureMode,
    relative_name: String,
    generated_relative_path: PathBuf,
    exposed_relative_path: PathBuf,
    contents: Vec<u8>,
    origin_layer: LayerKind,
    origin_detail: String,
}

#[derive(Debug, Clone)]
struct DriftedFile {
    exposed_relative_path: PathBuf,
    origin_layer: LayerKind,
    origin_detail: String,
    diff: String,
}

#[derive(Debug, Clone)]
struct CompositeSection {
    adapter: AdapterKind,
    kind: AssetKind,
    title: String,
    content: String,
    origin_layer: LayerKind,
    origin_detail: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct AssetMetadata {
    #[serde(default)]
    targets: Vec<String>,
}

struct PlanContext<'a> {
    project_root: &'a Path,
    adapter: AdapterKind,
    kind: AssetKind,
    origin_layer: LayerKind,
    origin_detail: String,
}

struct PlanState<'a> {
    plan: &'a mut Vec<PlannedFile>,
    sections: &'a mut Vec<CompositeSection>,
    seen: &'a mut BTreeMap<PathBuf, usize>,
}

const PLY_MANAGED_START: &str = "<!-- ply:start -->";
const PLY_MANAGED_END: &str = "<!-- ply:end -->";

fn ensure_managed_name(kind: AssetKind, name: &str) -> String {
    if !kind.requires_ply_prefix() || name.starts_with("ply-") {
        return name.to_string();
    }
    format!("ply-{name}")
}

fn managed_relative_path(kind: AssetKind, relative_path: &Path) -> Result<PathBuf> {
    if !kind.requires_ply_prefix() {
        return Ok(relative_path.to_path_buf());
    }

    let mut components = relative_path.components();
    let first = components
        .next()
        .ok_or_else(|| anyhow!("empty managed asset path"))?
        .as_os_str()
        .to_string_lossy()
        .to_string();
    let mut managed = PathBuf::from(ensure_managed_name(kind, &first));
    for component in components {
        managed.push(component.as_os_str());
    }
    Ok(managed)
}

#[derive(Debug, Clone, Copy)]
pub enum CommandTarget {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy)]
pub struct InitRequest {
    pub options: InitOptions,
    pub dry_run: bool,
    pub target: CommandTarget,
}

#[derive(Debug, Clone, Copy)]
pub struct ApplyOptions {
    pub dry_run: bool,
    pub yes: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CleanOptions {
    pub dry_run: bool,
    pub target: CommandTarget,
}

#[derive(Debug, Clone)]
pub struct CleanupPreview {
    pub items: Vec<String>,
    pub updates_git_excludes: bool,
    pub config_root: PathBuf,
    pub worktree_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CleanupReport {
    pub removed_items: Vec<String>,
    pub updated_git_excludes: bool,
    pub config_root: PathBuf,
    pub worktree_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InitReport {
    pub created_manifest: bool,
    pub created_local_fixture: bool,
    pub ignore_config: bool,
    pub adapters: Vec<String>,
    pub config_root: PathBuf,
    pub worktree_root: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct PackageInitRequest {
    pub name: String,
    pub path: PathBuf,
    pub kinds: Vec<AssetKind>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct PackageInitReport {
    pub target_root: PathBuf,
    pub created_paths: Vec<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct PackageDoctorRequest {
    pub path: PathBuf,
    pub fix: bool,
}

#[derive(Debug, Clone)]
pub struct PackageDoctorReport {
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct PackageGetRequest {
    pub path: PathBuf,
    pub key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PackageSetRequest {
    pub path: PathBuf,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub body: String,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub enum AddSourceLocation {
    Path(String),
    Git { repo: String, rev: Option<String> },
}

#[derive(Debug, Clone)]
pub enum AddSourceSshMode {
    None,
    DefaultKey,
    KeyPath(String),
}

#[derive(Debug, Clone)]
pub struct AddSourceRequest {
    pub id: String,
    pub location: AddSourceLocation,
    pub ssh: AddSourceSshMode,
}

#[derive(Debug, Clone)]
pub struct RemoveSourceRequest {
    pub id: String,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateSourcesRequest {
    pub source_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SourceMutationReport {
    pub body: String,
}

pub fn init_project(project_root: &Path, request: InitRequest) -> Result<InitReport> {
    let (config_root, worktree_root) = match request.target {
        CommandTarget::Project => {
            let context = git::repository_context(project_root)?;
            (context.config_root, context.worktree_root)
        }
        CommandTarget::Global => {
            let root = config::global_root()?;
            (root.clone(), root)
        }
    };

    let created_manifest = !config_root.join("ply.toml").exists();
    let created_local_fixture = !config_root
        .join("ply-packages")
        .join("example-review")
        .exists()
        && request.options.scaffold_local_packages;

    if !request.dry_run {
        fs::create_dir_all(worktree_root.join(".ply").join("generated"))?;
        fs::create_dir_all(
            config_root
                .join(".ply")
                .join("overlays")
                .join("codex")
                .join("skills"),
        )?;
        fs::create_dir_all(
            config_root
                .join(".ply")
                .join("overlays")
                .join("claude")
                .join("skills"),
        )?;
        if matches!(request.target, CommandTarget::Project) {
            git::ensure_local_excludes(&worktree_root, request.options)?;
        }
        config::write_default_manifest(&config_root, request.options)?;
        config::write_default_local_manifest(&config_root)?;
        config::write_state(
            &worktree_root,
            &State {
                schema_version: 1,
                ignore_config: request.options.ignore_config,
                owned_paths: Vec::new(),
            },
        )?;
        if config_root != worktree_root && !config_root.join(".ply").join("state.json").exists() {
            config::write_state(
                &config_root,
                &State {
                    schema_version: 1,
                    ignore_config: request.options.ignore_config,
                    owned_paths: Vec::new(),
                },
            )?;
        }
        if request.options.scaffold_local_packages {
            config::write_default_package_fixture(&config_root)?;
        }
    }

    Ok(InitReport {
        created_manifest,
        created_local_fixture,
        ignore_config: request.options.ignore_config,
        adapters: request
            .options
            .adapters
            .iter()
            .map(|adapter| adapter.to_string())
            .collect(),
        config_root,
        worktree_root,
        dry_run: request.dry_run,
    })
}

pub fn init_package(project_root: &Path, request: PackageInitRequest) -> Result<PackageInitReport> {
    let target_root = if request.path.is_absolute() {
        request.path.clone()
    } else {
        project_root.join(&request.path)
    };

    ensure_package_bootstrap_target(&target_root, &request.kinds)?;

    let mut created_paths = vec![PathBuf::from("ply-package.toml")];
    for kind in &request.kinds {
        created_paths.push(PathBuf::from(kind.as_str()));
    }

    if !request.dry_run {
        fs::create_dir_all(&target_root)?;
        config::write_package_manifest(&target_root, &request.name)?;
        for kind in &request.kinds {
            if kind.is_directory_based() {
                fs::create_dir_all(target_root.join(kind.as_str()))?;
            } else {
                fs::write(target_root.join("local-instructions.md"), "")?;
            }
        }
    }

    Ok(PackageInitReport {
        target_root,
        created_paths,
        dry_run: request.dry_run,
    })
}

pub fn doctor_package(
    project_root: &Path,
    request: PackageDoctorRequest,
) -> Result<PackageDoctorReport> {
    let package_root = if request.path.is_absolute() {
        request.path.clone()
    } else {
        project_root.join(&request.path)
    };

    let mut healthy = Vec::new();
    let mut applied = Vec::new();
    let mut suggestions = Vec::new();

    if request.fix && !package_root.exists() {
        fs::create_dir_all(&package_root)?;
        applied.push(ui::list_item(&format!(
            "created package root {}",
            package_root.display()
        )));
    }

    if !package_root.exists() {
        return Err(anyhow!(
            "{}",
            render_report_sections(&[
                (
                    "Issues",
                    vec![ui::status_line(
                        Tone::Error,
                        &format!("package root {} does not exist", package_root.display()),
                    )],
                ),
                ("Suggested fixes", suggestions),
            ])
        ));
    }

    if request.fix && !package_root.join("ply-package.toml").exists() {
        let package_name = prompt_required_package_name(&package_root)?;
        config::write_package_manifest(&package_root, &package_name)?;
        applied.push(ui::list_item("created ply-package.toml"));
    } else if !package_root.join("ply-package.toml").exists() {
        suggestions.push(ui::list_item(
            "run `ply doctor package --fix` to create ply-package.toml interactively",
        ));
    }

    healthy.push(ui::status_line(
        Tone::Success,
        &format!("package root {}", package_root.display()),
    ));

    let manifest_path = package_root.join("ply-package.toml");
    let manifest = match load_package_manifest(&package_root) {
        Ok(manifest) => {
            healthy.push(ui::status_line(Tone::Success, "ply-package.toml parsed"));
            manifest
        }
        Err(err) => {
            if request.fix {
                let mut editable =
                    load_package_manifest_for_edit(&package_root).map_err(|_| err)?;
                let changed = repair_package_manifest_interactively(&package_root, &mut editable)?;
                if changed {
                    config::write_package_manifest_contents(&package_root, &editable)?;
                    applied.push(ui::list_item("updated ply-package.toml interactively"));
                }
                match load_package_manifest(&package_root) {
                    Ok(manifest) => {
                        healthy.push(ui::status_line(Tone::Success, "ply-package.toml parsed"));
                        manifest
                    }
                    Err(err) => {
                        let issue_lines = vec![
                            ui::status_line(
                                Tone::Error,
                                &format!("failed to load {}", manifest_path.display()),
                            ),
                            ui::list_item(&err.to_string()),
                        ];
                        suggestions.push(ui::list_item(
                            "repair ply-package.toml manually, then rerun `ply doctor package`",
                        ));
                        return Err(anyhow!(
                            "{}",
                            render_report_sections(&[
                                ("Applied fixes", applied),
                                ("Healthy checks", healthy),
                                ("Issues", issue_lines),
                                ("Suggested fixes", suggestions),
                            ])
                        ));
                    }
                }
            } else {
                let issue_lines = vec![
                    ui::status_line(
                        Tone::Error,
                        &format!("failed to load {}", manifest_path.display()),
                    ),
                    ui::list_item(&err.to_string()),
                ];
                suggestions.push(ui::list_item(
                    "run `ply doctor package --fix` to repair supported metadata issues interactively",
                ));
                suggestions.push(ui::list_item(
                    "if ply-package.toml has TOML syntax errors, repair it manually first",
                ));
                return Err(anyhow!(
                    "{}",
                    render_report_sections(&[
                        ("Applied fixes", applied),
                        ("Healthy checks", healthy),
                        ("Issues", issue_lines),
                        ("Suggested fixes", suggestions),
                    ])
                ));
            }
        }
    };

    healthy.push(ui::status_line(
        Tone::Success,
        &format!("package name `{}`", manifest.name),
    ));
    if let Some(version) = manifest.version.as_deref() {
        healthy.push(ui::status_line(
            Tone::Success,
            &format!("version `{version}` is valid"),
        ));
    } else {
        suggestions.push(ui::list_item(
            "consider adding `version` to ply-package.toml when you want publishable package metadata",
        ));
    }
    if manifest.description.is_none() {
        suggestions.push(ui::list_item(
            "consider adding `description` to ply-package.toml for package discovery",
        ));
    }
    if manifest.license.is_none() {
        suggestions.push(ui::list_item(
            "consider adding `license` to ply-package.toml if the package will be shared",
        ));
    }

    let mut issue_lines = Vec::new();
    if let Err(err) = validate_package_root(&package_root, &manifest) {
        if request.fix
            && err
                .to_string()
                .contains("does not expose any supported managed assets")
        {
            let kinds = prompt_package_kinds()?;
            for kind in kinds {
                let path = package_asset_root(&package_root, kind);
                if path.exists() {
                    continue;
                }
                scaffold_package_kind(&package_root, kind)?;
                applied.push(ui::list_item(&format!("created {}", path.display())));
            }
            validate_package_root(&package_root, &manifest)?;
            healthy.push(ui::status_line(
                Tone::Success,
                "package root passed structure checks",
            ));
        } else {
            issue_lines.push(ui::status_line(Tone::Error, &err.to_string()));
            issue_lines.extend(package_fix_hints(&err.to_string(), request.fix));
        }
    } else {
        healthy.push(ui::status_line(
            Tone::Success,
            "package root passed structure checks",
        ));
    }

    if issue_lines.is_empty() {
        let resolved_package = ResolvedPackage {
            source_id: "package".to_string(),
            source_layer: LayerKind::Project,
            root: package_root.clone(),
            manifest: manifest.clone(),
        };
        let adapters = vec!["codex".to_string(), "claude".to_string()];
        match build_plan(package_root.as_path(), &adapters, &[resolved_package], &[]) {
            Ok(planned_files) => {
                healthy.push(ui::status_line(
                    Tone::Success,
                    &format!(
                        "{} managed file(s) planned for supported adapters",
                        planned_files.len()
                    ),
                ));
            }
            Err(err) => {
                issue_lines.push(ui::status_line(Tone::Error, &err.to_string()));
            }
        }
    }

    if !issue_lines.is_empty() {
        return Err(anyhow!(
            "{}",
            render_report_sections(&[
                ("Applied fixes", applied),
                ("Healthy checks", healthy),
                ("Issues", issue_lines),
                ("Suggested fixes", suggestions),
            ])
        ));
    }

    Ok(PackageDoctorReport {
        body: render_report_sections(&[
            ("Applied fixes", applied),
            ("Healthy checks", healthy),
            ("Suggested fixes", suggestions),
        ]),
    })
}

pub fn get_package_metadata(project_root: &Path, request: PackageGetRequest) -> Result<String> {
    let package_root = if request.path.is_absolute() {
        request.path.clone()
    } else {
        project_root.join(&request.path)
    };
    let manifest = load_package_manifest_for_edit(&package_root)?;
    match request.key.as_deref() {
        None => Ok(render_package_metadata(&manifest)),
        Some("name") => Ok(manifest.name),
        Some("version") => Ok(optional_string(manifest.version)),
        Some("description") => Ok(optional_string(manifest.description)),
        Some("license") => Ok(optional_string(manifest.license)),
        Some("targets") => Ok(render_targets(&manifest.targets)),
        Some(other) => Err(anyhow!(
            "unsupported package metadata key `{other}`; use one of name, version, description, license, targets"
        )),
    }
}

pub fn set_package_metadata(project_root: &Path, request: PackageSetRequest) -> Result<String> {
    let package_root = if request.path.is_absolute() {
        request.path.clone()
    } else {
        project_root.join(&request.path)
    };
    let mut manifest = load_package_manifest_for_edit(&package_root)?;
    match request.key.as_str() {
        "name" => manifest.name = request.value.trim().to_string(),
        "version" => manifest.version = parse_optional_value(&request.value),
        "description" => manifest.description = parse_optional_value(&request.value),
        "license" => manifest.license = parse_optional_value(&request.value),
        "targets" => manifest.targets = parse_targets_value(&request.value),
        other => {
            return Err(anyhow!(
                "unsupported package metadata key `{other}`; use one of name, version, description, license, targets"
            ));
        }
    }
    config::write_package_manifest_contents(&package_root, &manifest)?;
    Ok(render_package_metadata(&manifest))
}

pub fn add_source(
    project_root: &Path,
    request: AddSourceRequest,
    _target: CommandTarget,
) -> Result<SourceMutationReport> {
    let mut ssh_config = load_ssh_config_for_edit(project_root)?;
    let mut manifest = load_manifest_for_edit(project_root)?;
    let source = match request.location {
        AddSourceLocation::Path(path) => SourceConfig {
            id: request.id.clone(),
            kind: "path".to_string(),
            path: Some(path),
            repo: None,
            url: None,
            rev: None,
        },
        AddSourceLocation::Git { repo, rev } => SourceConfig {
            id: request.id.clone(),
            kind: "git".to_string(),
            path: None,
            repo: Some(repo),
            url: None,
            rev,
        },
    };

    if manifest
        .sources
        .iter()
        .any(|existing| existing.id == source.id)
    {
        return Err(anyhow!("duplicate source id `{}`", source.id));
    }

    update_ssh_config_for_added_source(&mut ssh_config, &request.id, &source, &request.ssh)?;

    let mut resolved_git_revision = None;
    if source.kind == "git" {
        let ssh_entry = ssh_config.sources.get(&source.id);
        let (_, revision) = git::clone_or_update_source(project_root, &source, ssh_entry)?;
        resolved_git_revision = Some(revision);
    }

    manifest.sources.push(source.clone());
    config::write_manifest(project_root, &manifest)?;
    config::write_ssh_config(project_root, &ssh_config)?;
    if let Some(revision) = resolved_git_revision.as_deref() {
        upsert_lockfile_source(
            project_root,
            LockedSource {
                id: source.id.clone(),
                kind: source.kind.clone(),
                path: source.path.clone(),
                repo: source.repo.clone(),
                resolved: revision.to_string(),
            },
        )?;
    }

    let mut body = format!("Updated {}", project_root.join("ply.toml").display());
    body.push_str("\n\nAdded:");
    body.push('\n');
    body.push_str(&ui::list_item(&render_source_summary(&source)));
    if !matches!(request.ssh, AddSourceSshMode::None) {
        body.push_str("\n\nSSH config:");
        body.push('\n');
        body.push_str(&ui::list_item("updated ply.ssh.toml for this source"));
    }

    if source.kind == "git" {
        body.push_str("\n\nLockfile:");
        body.push('\n');
        body.push_str(&ui::list_item(
            "refreshed ply.lock for the added Git source and preserved other locked Git revisions when present",
        ));
    }

    Ok(SourceMutationReport { body })
}

pub fn remove_source(
    project_root: &Path,
    request: RemoveSourceRequest,
) -> Result<SourceMutationReport> {
    let mut ssh_config = load_ssh_config_for_edit(project_root)?;
    let mut manifest = load_manifest_for_edit(project_root)?;
    let Some(index) = manifest
        .sources
        .iter()
        .position(|source| source.id == request.id)
    else {
        return Err(anyhow!("source `{}` is not configured", request.id));
    };
    let removed = manifest.sources.remove(index);
    config::write_manifest(project_root, &manifest)?;
    let removed_ssh = ssh_config.sources.remove(&request.id).is_some();
    if removed_ssh {
        config::write_ssh_config(project_root, &ssh_config)?;
    }

    let mut pruned_lock = false;
    if let Some(mut lockfile) = load_lockfile_if_present(project_root)? {
        let original_len = lockfile.sources.len();
        lockfile.sources.retain(|source| source.id != request.id);
        pruned_lock = lockfile.sources.len() != original_len;
        if pruned_lock {
            config::write_lockfile(project_root, &lockfile)?;
        }
    }

    let mut body = format!("Updated {}", project_root.join("ply.toml").display());
    body.push_str("\n\nRemoved:");
    body.push('\n');
    body.push_str(&ui::list_item(&render_source_summary(&removed)));
    if request.force {
        body.push_str("\n\nFlags:");
        body.push('\n');
        body.push_str(&ui::list_item(
            "`--force` is accepted for future compatibility; no source-reference override was needed in this source-only model",
        ));
    }
    if pruned_lock {
        body.push_str("\n\nLockfile:");
        body.push('\n');
        body.push_str(&ui::list_item(
            "removed the stale source entry from ply.lock",
        ));
    }
    if removed_ssh {
        body.push_str("\n\nSSH config:");
        body.push('\n');
        body.push_str(&ui::list_item(
            "removed the stale source entry from ply.ssh.toml",
        ));
    }

    Ok(SourceMutationReport { body })
}

pub fn update_sources(
    project_root: &Path,
    request: UpdateSourcesRequest,
    target: CommandTarget,
) -> Result<SourceMutationReport> {
    let root = target_root(project_root, target)?;
    let layer = match target {
        CommandTarget::Project => LayerKind::Project,
        CommandTarget::Global => LayerKind::Global,
    };
    let composition = compose_single_root(&root, layer)?;
    let previous_lock = load_lockfile_if_present(&root)?;
    let resolved_sources =
        resolve_sources_for_update(&root, &composition, &request, previous_lock)?;
    let updated_git_ids = collect_updated_git_ids(&composition, &request)?;

    let lockfile = Lockfile {
        schema_version: 1,
        sources: resolved_sources
            .iter()
            .map(|source| LockedSource {
                id: source.id.clone(),
                kind: source.kind.clone(),
                path: source.locator_path.clone(),
                repo: source.locator_repo.clone(),
                resolved: source.resolved.clone(),
            })
            .collect(),
    };
    config::write_lockfile(&root, &lockfile)?;

    let mut body = format!("Updated {}", root.join("ply.lock").display());
    if updated_git_ids.is_empty() {
        body.push_str("\n\nNo Git sources were configured.");
    } else {
        body.push_str("\n\nRefreshed Git sources:");
        for id in updated_git_ids {
            body.push('\n');
            body.push_str(&ui::list_item(&id));
        }
    }

    Ok(SourceMutationReport { body })
}

fn upsert_lockfile_source(project_root: &Path, source: LockedSource) -> Result<()> {
    let mut lockfile = load_lockfile_if_present(project_root)?.unwrap_or(Lockfile {
        schema_version: 1,
        sources: Vec::new(),
    });
    if let Some(existing) = lockfile
        .sources
        .iter_mut()
        .find(|entry| entry.id == source.id)
    {
        *existing = source;
    } else {
        lockfile.sources.push(source);
    }
    config::write_lockfile(project_root, &lockfile)
}

fn locked_source_matches_config(locked: &LockedSource, source: &SourceConfig) -> bool {
    if locked.kind != source.kind {
        return false;
    }
    match source.kind.as_str() {
        "path" => locked.path == source.path,
        "git" => locked.repo == source.repo.clone().or_else(|| source.url.clone()),
        _ => false,
    }
}

fn default_package_name(package_root: &Path) -> String {
    package_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("package")
        .to_string()
}

fn package_asset_root(package_root: &Path, kind: AssetKind) -> PathBuf {
    match kind {
        AssetKind::LocalInstructions => package_root.join("local-instructions.md"),
        _ => package_root.join(kind.as_str()),
    }
}

fn scaffold_package_kind(package_root: &Path, kind: AssetKind) -> Result<()> {
    let target = package_asset_root(package_root, kind);
    if kind.is_directory_based() {
        fs::create_dir_all(target)?;
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if !target.exists() {
            fs::write(target, "")?;
        }
    }
    Ok(())
}

fn package_fix_hints(error: &str, fix_mode: bool) -> Vec<String> {
    let mut hints = Vec::new();
    if error.contains("does not expose any supported managed assets") {
        if fix_mode {
            hints.push(ui::list_item(
                "answer the scaffold prompt to create supported asset roots",
            ));
        } else {
            hints.push(ui::list_item(
                "run `ply doctor package --fix` to scaffold supported asset roots interactively",
            ));
        }
    }
    if error.contains("contains unsupported adapter directory") {
        hints.push(ui::list_item(
            "move authored content into portable asset roots like `skills/`, `commands/`, or `agents/`",
        ));
    }
    if error.contains("only allows [") {
        hints.push(ui::list_item(
            "align resource-level `targets` with the package-level `targets` declared in ply-package.toml",
        ));
    }
    hints
}

fn prompt_required_package_name(package_root: &Path) -> Result<String> {
    let default_name = default_package_name(package_root);
    loop {
        let value = ui::prompt_text(
            "Package name required",
            "Ply needs a package name to create ply-package.toml.",
            &format!("Package name [{default_name}]: "),
        )
        .map_err(|err| anyhow!("failed to read package name: {err}"))?;
        let resolved = if value.trim().is_empty() {
            default_name.clone()
        } else {
            value.trim().to_string()
        };
        if resolved.trim().is_empty() {
            continue;
        }
        return Ok(resolved);
    }
}

fn prompt_package_kinds() -> Result<Vec<AssetKind>> {
    loop {
        let value = ui::prompt_text(
            "Package assets required",
            "This package root does not expose any supported managed assets. Choose one or more kinds to scaffold.",
            "Kinds [skills,commands]: ",
        )
        .map_err(|err| anyhow!("failed to read package kinds: {err}"))?;
        let raw = if value.trim().is_empty() {
            "skills,commands".to_string()
        } else {
            value
        };
        let mut kinds = Vec::new();
        let mut valid = true;
        for item in raw.split(',') {
            match AssetKind::parse(item.trim()) {
                Ok(kind) => kinds.push(kind),
                Err(_) => {
                    valid = false;
                    break;
                }
            }
        }
        if valid && !kinds.is_empty() {
            return Ok(kinds);
        }
    }
}

fn repair_package_manifest_interactively(
    package_root: &Path,
    manifest: &mut PackageManifest,
) -> Result<bool> {
    let mut changed = false;
    if manifest.name.trim().is_empty() {
        manifest.name = prompt_required_package_name(package_root)?;
        changed = true;
    }
    if let Some(version) = manifest.version.clone()
        && semver::Version::parse(&version).is_err()
    {
        let input = ui::prompt_text(
            "Package version is invalid",
            &format!("Current version `{version}` is not valid semver. Enter a new version or leave empty to clear it."),
            "Version: ",
        )
        .map_err(|err| anyhow!("failed to read package version: {err}"))?;
        manifest.version = parse_optional_value(&input);
        changed = true;
    }
    let invalid_targets = manifest
        .targets
        .iter()
        .any(|target| AdapterKind::parse(target).is_err());
    if invalid_targets {
        let input = ui::prompt_text(
            "Package targets are invalid",
            "Targets must be a comma-separated subset of codex,claude. Leave empty to clear targets.",
            "Targets: ",
        )
        .map_err(|err| anyhow!("failed to read package targets: {err}"))?;
        manifest.targets = parse_targets_value(&input);
        changed = true;
    }
    Ok(changed)
}

fn parse_optional_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_targets_value(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn optional_string(value: Option<String>) -> String {
    value.unwrap_or_default()
}

fn render_targets(targets: &[String]) -> String {
    if targets.is_empty() {
        String::new()
    } else {
        targets.join(",")
    }
}

fn render_package_metadata(manifest: &PackageManifest) -> String {
    let mut lines = Vec::new();
    lines.push(format!("name={}", manifest.name));
    lines.push(format!(
        "version={}",
        optional_string(manifest.version.clone())
    ));
    lines.push(format!(
        "description={}",
        optional_string(manifest.description.clone())
    ));
    lines.push(format!(
        "license={}",
        optional_string(manifest.license.clone())
    ));
    lines.push(format!("targets={}", render_targets(&manifest.targets)));
    lines.join("\n")
}

fn ensure_package_bootstrap_target(target_root: &Path, kinds: &[AssetKind]) -> Result<()> {
    if !target_root.exists() {
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(target_root)? {
        entries.push(entry?);
    }

    let allowed_names = [
        ".git",
        ".gitignore",
        "README",
        "README.md",
        "LICENSE",
        "LICENSE.md",
    ];
    for entry in &entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !allowed_names.contains(&name.as_ref()) {
            return Err(anyhow!(
                "refusing to initialize package in non-empty directory {}; found `{}`",
                target_root.display(),
                name
            ));
        }
    }

    if target_root.join("ply-package.toml").exists() {
        return Err(anyhow!(
            "refusing to initialize package in {}; ply-package.toml already exists",
            target_root.display()
        ));
    }

    for kind in kinds {
        let path = if kind.is_directory_based() {
            target_root.join(kind.as_str())
        } else {
            target_root.join("local-instructions.md")
        };
        if path.exists() {
            return Err(anyhow!(
                "refusing to initialize package in {}; target path `{}` already exists",
                target_root.display(),
                path.strip_prefix(target_root).unwrap_or(&path).display()
            ));
        }
    }

    Ok(())
}

pub fn preview_cleanup(project_root: &Path, options: CleanOptions) -> Result<CleanupPreview> {
    let project_context = matches!(options.target, CommandTarget::Project)
        .then(|| git::repository_context(project_root))
        .transpose()?;
    let (root, config_root, worktree_root, linked_shared_config) = match &project_context {
        Some(context) => (
            context.worktree_root.clone(),
            context.config_root.clone(),
            context.worktree_root.clone(),
            context.uses_main_worktree_config(),
        ),
        None => {
            let root = config::global_root()?;
            (root.clone(), root.clone(), root, false)
        }
    };

    let mut items = Vec::new();
    let cleanup_paths = if linked_shared_config {
        managed_output_cleanup_paths(&root)?
    } else {
        managed_cleanup_paths(&root)?
    };
    for path in cleanup_paths {
        items.push(path.strip_prefix(&root)?.display().to_string());
    }
    let updates_git_excludes = matches!(options.target, CommandTarget::Project)
        && !linked_shared_config
        && git::has_ply_excludes(&root);

    if items.is_empty() && !updates_git_excludes {
        let hint = if matches!(options.target, CommandTarget::Global) {
            "ply init -g"
        } else {
            "ply init"
        };
        return Err(anyhow!(
            "ply is not initialized in {}; run `{hint}` to scaffold ply.toml and local state files",
            root.display()
        ));
    }

    Ok(CleanupPreview {
        items,
        updates_git_excludes,
        config_root,
        worktree_root,
    })
}

pub fn clean_project(project_root: &Path, options: CleanOptions) -> Result<CleanupReport> {
    let preview = preview_cleanup(project_root, options)?;
    let root = preview.worktree_root.clone();
    let linked_shared_config = preview.config_root != preview.worktree_root;
    if options.dry_run {
        return Ok(CleanupReport {
            removed_items: preview.items,
            updated_git_excludes: preview.updates_git_excludes,
            config_root: preview.config_root,
            worktree_root: preview.worktree_root,
        });
    }

    let mut removed_items = Vec::new();
    let cleanup_paths = if linked_shared_config {
        managed_output_cleanup_paths(&root)?
    } else {
        managed_cleanup_paths(&root)?
    };
    for path in cleanup_paths {
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else if path.exists() {
            fs::remove_file(&path)?;
        }
        removed_items.push(path.strip_prefix(&root)?.display().to_string());
        prune_empty_parents(&root, &path)?;
    }

    let updated_git_excludes = if preview.updates_git_excludes {
        git::remove_local_excludes(&root)?
    } else {
        false
    };

    Ok(CleanupReport {
        removed_items,
        updated_git_excludes,
        config_root: preview.config_root,
        worktree_root: preview.worktree_root,
    })
}

pub fn apply(project_root: &Path, options: ApplyOptions) -> Result<ApplyReport> {
    let context = git::repository_context(project_root)?;
    let previous_state = load_state(&context.worktree_root)?;
    let config_state = load_state(&context.config_root)?;
    let previous_lock = load_lockfile_if_present(&context.config_root)?;
    git::ensure_local_excludes(
        &context.worktree_root,
        InitOptions {
            scaffold_local_packages: false,
            ignore_config: config_state.ignore_config,
            adapters: &["codex", "claude"],
        },
    )?;
    let composition = compose_project_config(&context.worktree_root)?;
    let (sources, packages) =
        resolve_composed_for_apply(&context.config_root, &composition, previous_lock)?;
    let planned_files = build_plan(
        &context.worktree_root,
        &composition.adapters,
        &packages,
        &composition.overlays,
    )?;
    verify_exposed_targets(&context.worktree_root, &planned_files, &previous_state)?;
    let drifted = collect_exposed_drifts(&context.worktree_root, &planned_files, &previous_state)?;

    if options.dry_run {
        let body = prepend_project_context(
            render_apply_dry_run(&composition, &sources, &packages, &planned_files, &drifted),
            &context,
        );
        return Ok(ApplyReport {
            body,
            dry_run: true,
        });
    }

    let approvals = resolve_drift_approvals(&drifted, options.yes)?;
    let approved_paths: BTreeSet<PathBuf> = approvals
        .iter()
        .filter_map(|(path, approved)| approved.then_some(path.clone()))
        .collect();
    let skipped_files = approvals.values().filter(|approved| !**approved).count();

    write_generated_tree(&context.worktree_root, &planned_files)?;
    remove_stale_paths(&context.worktree_root, &previous_state, &planned_files)?;
    write_exposed_tree(&context.worktree_root, &planned_files, &approved_paths)?;
    let lockfile = Lockfile {
        schema_version: 1,
        sources: sources
            .iter()
            .map(|source| LockedSource {
                id: source.id.clone(),
                kind: source.kind.clone(),
                path: source.locator_path.clone(),
                repo: source.locator_repo.clone(),
                resolved: source.resolved.clone(),
            })
            .collect(),
    };
    config::write_lockfile(&context.config_root, &lockfile)?;
    let state = State {
        schema_version: 1,
        ignore_config: config_state.ignore_config,
        owned_paths: planned_files
            .iter()
            .map(|file| OwnedPath {
                adapter: file.adapter.as_str().to_string(),
                kind: file.kind.as_str().to_string(),
                exposure_mode: file.exposure_mode.as_str().to_string(),
                relative_name: file.relative_name.clone(),
                generated_path: file.generated_relative_path.to_string_lossy().to_string(),
                exposed_path: file.exposed_relative_path.to_string_lossy().to_string(),
                generated_digest: content_digest(&file.contents),
                exposed_digest: content_digest(&file.contents),
            })
            .collect(),
    };
    config::write_state(&context.worktree_root, &state)?;

    let mut body = format!(
        "Applied {} managed file(s) across {} source(s).",
        planned_files.len(),
        sources.len()
    );
    if drifted.is_empty() {
        body.push_str("\n\nNo managed drifts needed consent.");
    } else {
        body.push_str(&format!(
            "\n\n{} drifted managed file(s) detected.",
            drifted.len()
        ));
        if skipped_files > 0 {
            body.push_str(&format!("\n{} file(s) were left unchanged.", skipped_files));
        }
    }

    Ok(ApplyReport {
        body: prepend_project_context(body, &context),
        dry_run: false,
    })
}

pub fn diff(project_root: &Path) -> Result<String> {
    let context = git::repository_context(project_root)?;
    let output_root = &context.worktree_root;
    let composition = compose_project_config(output_root)?;
    let (_, packages) = resolve_composed(&composition)?;
    let previous_state = load_state(output_root)?;
    let planned_files = build_plan(
        output_root,
        &composition.adapters,
        &packages,
        &composition.overlays,
    )?;

    let desired_generated: BTreeSet<PathBuf> = planned_files
        .iter()
        .map(|file| generated_abs_path(output_root, file))
        .collect();
    let desired_exposed: BTreeSet<PathBuf> = planned_files
        .iter()
        .map(|file| exposed_abs_path(output_root, file))
        .collect();
    let owned_previous: BTreeSet<PathBuf> = previous_state
        .owned_paths
        .iter()
        .map(|owned| output_root.join(&owned.exposed_path))
        .collect();

    let mut generated_changes = Vec::new();
    let mut exposed_changes = Vec::new();
    let mut stale_paths = Vec::new();
    let mut safety_violations = Vec::new();
    for file in &planned_files {
        let generated = generated_abs_path(output_root, file);
        let exposed = exposed_abs_path(output_root, file);
        if !generated.exists() {
            generated_changes.push(ui::status_line(
                Tone::Info,
                &format!("generate {}", file.generated_relative_path.display()),
            ));
        }
        if git::is_tracked(output_root, &exposed)? {
            safety_violations.push(ui::status_line(
                Tone::Warning,
                &format!("tracked target {}", file.exposed_relative_path.display()),
            ));
        }
        if !git::is_ignored(output_root, &exposed)? {
            safety_violations.push(ui::status_line(
                Tone::Warning,
                &format!("unignored target {}", file.exposed_relative_path.display()),
            ));
        }
        if !exposed.exists() {
            exposed_changes.push(ui::status_line(
                Tone::Info,
                &format!("expose {}", file.exposed_relative_path.display()),
            ));
            continue;
        }
        let current = fs::read(&exposed)?;
        if current != file.contents {
            exposed_changes.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "drift {} (desired from {}: {})",
                    file.exposed_relative_path.display(),
                    file.origin_layer.as_str(),
                    file.origin_detail
                ),
            ));
            let diff = git::diff_contents(
                &current,
                &file.contents,
                &file.exposed_relative_path.to_string_lossy(),
            )?;
            if !diff.is_empty() {
                exposed_changes.push(diff);
            }
        }
    }

    for stale in owned_previous.difference(&desired_exposed) {
        stale_paths.push(ui::status_line(
            Tone::Warning,
            &format!("remove {}", stale.strip_prefix(output_root)?.display()),
        ));
    }
    for generated_path in collect_file_paths(&output_root.join(".ply").join("generated"))? {
        if !desired_generated.contains(&generated_path) {
            stale_paths.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "remove {}",
                    generated_path.strip_prefix(output_root)?.display()
                ),
            ));
        }
    }

    let rendered = render_report_sections(&[
        ("Generated changes", generated_changes),
        ("Exposed changes", exposed_changes),
        ("Stale managed paths", stale_paths),
        ("Safety violations", safety_violations),
    ]);
    if rendered.is_empty() {
        return Ok(prepend_project_context(
            "no differences".to_string(),
            &context,
        ));
    }
    Ok(prepend_project_context(rendered, &context))
}

pub fn doctor(project_root: &Path, target: CommandTarget) -> Result<String> {
    let project_context = matches!(target, CommandTarget::Project)
        .then(|| git::repository_context(project_root))
        .transpose()?;
    let root = if let Some(context) = &project_context {
        context.worktree_root.clone()
    } else {
        config::global_root()?
    };
    let composition = match &project_context {
        Some(context) => compose_project_config(&context.worktree_root)?,
        None => compose_single_root(&root, LayerKind::Global)?,
    };
    let (sources, packages) = resolve_composed(&composition)?;
    let planned_files = build_plan(
        &root,
        &composition.adapters,
        &packages,
        &composition.overlays,
    )?;
    let previous_state = load_state(&root)?;
    let mut healthy = vec![ui::status_line(Tone::Success, "manifest parsed")];
    healthy.push(ui::status_line(
        Tone::Success,
        &format!("{} source(s) resolved", sources.len()),
    ));
    healthy.push(ui::status_line(
        Tone::Success,
        &format!("{} package(s) resolved", packages.len()),
    ));
    healthy.push(ui::status_line(
        Tone::Success,
        &format!("{} managed file(s) planned", planned_files.len()),
    ));
    let mut warnings = Vec::new();
    if !git::has_ply_excludes(&root) {
        warnings.push(ui::status_line(
            Tone::Warning,
            "missing Ply block in .git/info/exclude",
        ));
    }
    for file in &planned_files {
        let exposed = exposed_abs_path(&root, file);
        if git::is_tracked(&root, &exposed)? {
            warnings.push(ui::status_line(
                Tone::Warning,
                &format!("tracked target {}", file.exposed_relative_path.display()),
            ));
        }
        if !git::is_ignored(&root, &exposed)? {
            warnings.push(ui::status_line(
                Tone::Warning,
                &format!("unignored target {}", file.exposed_relative_path.display()),
            ));
        }
    }
    for owned in &previous_state.owned_paths {
        let generated = root.join(&owned.generated_path);
        if !generated.exists() {
            warnings.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "state points to missing generated path {}",
                    owned.generated_path
                ),
            ));
        }
        let exposed = root.join(&owned.exposed_path);
        if !exposed.exists() {
            warnings.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "state points to missing exposed path {}",
                    owned.exposed_path
                ),
            ));
        }
    }
    let report = render_report_sections(&[("Healthy checks", healthy), ("Warnings", warnings)]);
    Ok(match project_context {
        Some(context) => prepend_project_context(report, &context),
        None => report,
    })
}

pub fn list_packages(project_root: &Path, target: CommandTarget) -> Result<String> {
    let project_context = matches!(target, CommandTarget::Project)
        .then(|| git::repository_context(project_root))
        .transpose()?;
    let composition = match &project_context {
        Some(context) => compose_project_config(&context.worktree_root)?,
        None => compose_single_root(&config::global_root()?, LayerKind::Global)?,
    };
    let (_, packages) = resolve_composed(&composition)?;
    let mut lines = Vec::new();
    for package in packages {
        lines.push(ui::list_item(&format!(
            "{} ({}) from source {} [{}]",
            package.manifest.name,
            package
                .manifest
                .version
                .unwrap_or_else(|| "unversioned".to_string()),
            package.source_id,
            package.source_layer.as_str()
        )));
    }
    if lines.is_empty() {
        lines.push(ui::list_item("No packages configured."));
    }
    let report = lines.join("\n");
    Ok(match project_context {
        Some(context) => prepend_project_context(report, &context),
        None => report,
    })
}

pub fn list_sources(project_root: &Path, target: CommandTarget) -> Result<String> {
    let project_context = matches!(target, CommandTarget::Project)
        .then(|| git::repository_context(project_root))
        .transpose()?;
    let composition = match &project_context {
        Some(context) => compose_project_config(&context.worktree_root)?,
        None => compose_single_root(&config::global_root()?, LayerKind::Global)?,
    };
    let (sources, _) = resolve_composed(&composition)?;
    let mut lines = Vec::new();
    for source in sources {
        lines.push(ui::list_item(&format!(
            "{} [{}] {} ({})",
            source.id,
            source.kind,
            source.resolved,
            source.layer.as_str()
        )));
    }
    if lines.is_empty() {
        lines.push(ui::list_item("No sources configured."));
    }
    let report = lines.join("\n");
    Ok(match project_context {
        Some(context) => prepend_project_context(report, &context),
        None => report,
    })
}

fn render_source_summary(source: &SourceConfig) -> String {
    match source.kind.as_str() {
        "path" => format!(
            "{} [path] {}",
            source.id,
            source.path.as_deref().unwrap_or("<missing-path>")
        ),
        "git" => {
            let repo = source
                .repo
                .as_deref()
                .or(source.url.as_deref())
                .unwrap_or("<missing-repo>");
            match source.rev.as_deref() {
                Some(rev) => format!("{} [git] {} @ {}", source.id, repo, rev),
                None => format!("{} [git] {}", source.id, repo),
            }
        }
        _ => format!("{} [{}]", source.id, source.kind),
    }
}

fn update_ssh_config_for_added_source(
    ssh_config: &mut SshConfigFile,
    source_id: &str,
    source: &SourceConfig,
    mode: &AddSourceSshMode,
) -> Result<()> {
    match mode {
        AddSourceSshMode::None => Ok(()),
        AddSourceSshMode::DefaultKey => {
            if source.kind != "git" {
                return Err(anyhow!(
                    "`--ssh` is only supported when adding a Git source"
                ));
            }
            ssh_config.sources.insert(
                source_id.to_string(),
                SshSourceConfig {
                    use_ssh: true,
                    ssh_key_path: None,
                    ssh_key_env: None,
                },
            );
            Ok(())
        }
        AddSourceSshMode::KeyPath(path) => {
            if source.kind != "git" {
                return Err(anyhow!(
                    "`--ssh-key` is only supported when adding a Git source"
                ));
            }
            ssh_config.sources.insert(
                source_id.to_string(),
                SshSourceConfig {
                    use_ssh: true,
                    ssh_key_path: Some(path.clone()),
                    ssh_key_env: None,
                },
            );
            Ok(())
        }
    }
}

fn collect_updated_git_ids(
    composition: &ComposedConfig,
    request: &UpdateSourcesRequest,
) -> Result<Vec<String>> {
    match request.source_id.as_deref() {
        Some(source_id) => {
            let source = composition
                .sources
                .iter()
                .find(|source| source.config.id == source_id)
                .ok_or_else(|| anyhow!("source `{source_id}` is not configured"))?;
            if source.config.kind != "git" {
                return Err(anyhow!(
                    "source `{source_id}` is a `{}` source; only Git sources can be refreshed",
                    source.config.kind
                ));
            }
            Ok(vec![source_id.to_string()])
        }
        None => Ok(composition
            .sources
            .iter()
            .filter(|source| source.config.kind == "git")
            .map(|source| source.config.id.clone())
            .collect()),
    }
}

fn target_root(project_root: &Path, target: CommandTarget) -> Result<PathBuf> {
    match target {
        CommandTarget::Project => Ok(git::repository_context(project_root)?.config_root),
        CommandTarget::Global => config::global_root(),
    }
}

fn project_context_lines(context: &git::RepositoryContext) -> Vec<String> {
    let config_origin = if context.uses_main_worktree_config() {
        "main worktree"
    } else {
        "active worktree"
    };
    vec![
        ui::status_line(
            Tone::Info,
            &format!(
                "configuration ({config_origin}): {}",
                context.config_root.display()
            ),
        ),
        ui::status_line(
            Tone::Info,
            &format!("active worktree: {}", context.worktree_root.display()),
        ),
    ]
}

fn prepend_project_context(body: String, context: &git::RepositoryContext) -> String {
    render_report_sections(&[("Repository context", project_context_lines(context))])
        + if body.is_empty() { "" } else { "\n\n" }
        + &body
}

fn compose_project_config(project_root: &Path) -> Result<ComposedConfig> {
    let config_root = git::repository_context(project_root)?.config_root;
    let project_manifest = load_manifest(&config_root)?;
    let project_local_manifest = load_local_manifest_if_present(&config_root)?;
    let project_ssh_config = load_ssh_config_if_present(&config_root)?.unwrap_or_default();
    let project_overlays = load_local_overlays(&config_root)?;
    let project_manifest = merge_local_manifest(project_manifest, project_local_manifest)?;
    let mut layers = Vec::new();

    if project_manifest.install.use_global {
        let global_root = config::global_root()?;
        if let Some(global_manifest) = load_manifest_if_present(&global_root)? {
            let local_manifest = load_local_manifest_if_present(&global_root)?;
            let ssh_config = load_ssh_config_if_present(&global_root)?.unwrap_or_default();
            let global_manifest = merge_local_manifest(global_manifest, local_manifest)?;
            let overlays = load_local_overlays(&global_root)?;
            layers.push(LayerConfig {
                kind: LayerKind::Global,
                root: global_root,
                manifest: global_manifest,
                ssh_config,
                overlays,
            });
        }
    }

    layers.push(LayerConfig {
        kind: LayerKind::Project,
        root: config_root,
        manifest: project_manifest,
        ssh_config: project_ssh_config,
        overlays: project_overlays,
    });

    compose_layers(layers)
}

fn compose_single_root(root: &Path, layer: LayerKind) -> Result<ComposedConfig> {
    let hint = if matches!(layer, LayerKind::Global) {
        "ply init -g"
    } else {
        "ply init"
    };
    config::ensure_initialized_with_hint(root, hint)?;
    let manifest = load_manifest(root)?;
    let local_manifest = load_local_manifest_if_present(root)?;
    let manifest = merge_local_manifest(manifest, local_manifest)?;
    let ssh_config = load_ssh_config_if_present(root)?.unwrap_or_default();
    let overlays = load_local_overlays(root)?;
    compose_layers(vec![LayerConfig {
        kind: layer,
        root: root.to_path_buf(),
        manifest,
        ssh_config,
        overlays,
    }])
}

fn compose_layers(layers: Vec<LayerConfig>) -> Result<ComposedConfig> {
    let mut adapters = Vec::new();
    let mut seen_adapters = BTreeSet::new();
    let mut source_index = BTreeMap::new();
    let mut sources = Vec::new();
    let mut overlays = Vec::new();

    for layer in layers {
        for adapter in &layer.manifest.adapters {
            if seen_adapters.insert(adapter.clone()) {
                adapters.push(adapter.clone());
            }
        }

        for source in &layer.manifest.sources {
            if let Some(index) = source_index.insert(source.id.clone(), sources.len()) {
                sources[index] = MergedSource {
                    config: source.clone(),
                    ssh_config: layer.ssh_config.sources.get(&source.id).cloned(),
                    root: layer.root.clone(),
                    layer: layer.kind,
                };
            } else {
                sources.push(MergedSource {
                    config: source.clone(),
                    ssh_config: layer.ssh_config.sources.get(&source.id).cloned(),
                    root: layer.root.clone(),
                    layer: layer.kind,
                });
            }
        }

        for overlay in &layer.overlays.overlays {
            overlays.push(MergedOverlay {
                entry: overlay.clone(),
                root: layer.root.clone(),
                layer: layer.kind,
            });
        }
    }

    Ok(ComposedConfig {
        adapters,
        sources,
        overlays,
    })
}

fn resolve_sources_for_update(
    project_root: &Path,
    config: &ComposedConfig,
    request: &UpdateSourcesRequest,
    previous_lock: Option<Lockfile>,
) -> Result<Vec<ResolvedSource>> {
    let previous_by_id = previous_lock
        .unwrap_or_default()
        .sources
        .into_iter()
        .map(|source| (source.id.clone(), source))
        .collect::<BTreeMap<_, _>>();
    let target_id = request.source_id.as_deref();
    let mut found_target = target_id.is_none();
    let mut sources = Vec::new();

    for source in &config.sources {
        let resolved = match source.config.kind.as_str() {
            "path" => resolve_path_source(source)?,
            "git" => {
                if target_id.is_none() || target_id == Some(source.config.id.as_str()) {
                    found_target = true;
                    resolve_git_source(source)?
                } else {
                    let Some(previous) = previous_by_id.get(&source.config.id) else {
                        return Err(anyhow!(
                            "cannot refresh only source `{}` because ply.lock does not contain a previous revision for Git source `{}`; run `ply update` without a source id first",
                            target_id.unwrap_or_default(),
                            source.config.id
                        ));
                    };
                    if previous.kind != source.config.kind {
                        return Err(anyhow!(
                            "cannot preserve locked revision for source `{}` because ply.lock recorded kind `{}` but the current manifest uses `{}`",
                            source.config.id,
                            previous.kind,
                            source.config.kind
                        ));
                    }
                    if !locked_source_matches_config(previous, &source.config) {
                        return Err(anyhow!(
                            "cannot preserve locked revision for source `{}` because ply.lock recorded a different source locator; run `ply update` without a source id first",
                            source.config.id
                        ));
                    }
                    ResolvedSource {
                        id: source.config.id.clone(),
                        kind: source.config.kind.clone(),
                        resolved: previous.resolved.clone(),
                        root: git_source_root(project_root, source),
                        locator_path: None,
                        locator_repo: source
                            .config
                            .repo
                            .clone()
                            .or_else(|| source.config.url.clone()),
                        layer: source.layer,
                    }
                }
            }
            other => return Err(anyhow!("unsupported source kind `{other}`")),
        };
        sources.push(resolved);
    }

    if !found_target {
        let target_id = target_id.expect("target id should exist when not found");
        return Err(anyhow!("source `{target_id}` is not configured"));
    }

    Ok(sources)
}

fn resolve_path_source(source: &MergedSource) -> Result<ResolvedSource> {
    let path = source.root.join(
        source
            .config
            .path
            .as_deref()
            .ok_or_else(|| anyhow!("path source `{}` missing path", source.config.id))?,
    );
    let root = path.canonicalize().with_context(|| {
        format!(
            "failed to resolve path source `{}` at {}",
            source.config.id,
            path.display()
        )
    })?;
    Ok(ResolvedSource {
        id: source.config.id.clone(),
        kind: source.config.kind.clone(),
        resolved: root.display().to_string(),
        root,
        locator_path: source.config.path.clone(),
        locator_repo: None,
        layer: source.layer,
    })
}

fn resolve_git_source(source: &MergedSource) -> Result<ResolvedSource> {
    let (root, revision) =
        git::clone_or_update_source(&source.root, &source.config, source.ssh_config.as_ref())?;
    Ok(ResolvedSource {
        id: source.config.id.clone(),
        kind: source.config.kind.clone(),
        resolved: revision,
        root,
        locator_path: None,
        locator_repo: source
            .config
            .repo
            .clone()
            .or_else(|| source.config.url.clone()),
        layer: source.layer,
    })
}

fn resolve_locked_git_source(
    source: &MergedSource,
    locked: &LockedSource,
) -> Result<ResolvedSource> {
    let root = git::ensure_source_at_revision(
        &source.root,
        &source.config,
        source.ssh_config.as_ref(),
        &locked.resolved,
    )?;
    Ok(ResolvedSource {
        id: source.config.id.clone(),
        kind: source.config.kind.clone(),
        resolved: locked.resolved.clone(),
        root,
        locator_path: None,
        locator_repo: source
            .config
            .repo
            .clone()
            .or_else(|| source.config.url.clone()),
        layer: source.layer,
    })
}

fn git_source_root(project_root: &Path, source: &MergedSource) -> PathBuf {
    let _ = project_root;
    git::source_checkout_root(&source.root, &source.config).unwrap_or_else(|_| {
        source
            .root
            .join(".ply")
            .join("cache")
            .join("sources")
            .join(&source.config.id)
    })
}

fn resolve_composed(
    config: &ComposedConfig,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let mut sources = Vec::new();
    for source in &config.sources {
        let resolved = match source.config.kind.as_str() {
            "path" => resolve_path_source(source)?,
            "git" => resolve_git_source(source)?,
            other => return Err(anyhow!("unsupported source kind `{other}`")),
        };
        sources.push(resolved);
    }
    finalize_resolved_packages(sources)
}

fn resolve_composed_for_apply(
    project_root: &Path,
    config: &ComposedConfig,
    previous_lock: Option<Lockfile>,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let previous_by_id = previous_lock
        .unwrap_or_default()
        .sources
        .into_iter()
        .map(|source| (source.id.clone(), source))
        .collect::<BTreeMap<_, _>>();
    let mut sources = Vec::new();
    for source in &config.sources {
        let resolved = match source.config.kind.as_str() {
            "path" => resolve_path_source(source)?,
            "git" => match previous_by_id.get(&source.config.id) {
                Some(locked) if locked_source_matches_config(locked, &source.config) => {
                    resolve_locked_git_source(source, locked)?
                }
                _ => resolve_git_source(source)?,
            },
            other => return Err(anyhow!("unsupported source kind `{other}`")),
        };
        sources.push(resolved);
    }
    let _ = project_root;
    finalize_resolved_packages(sources)
}

fn finalize_resolved_packages(
    sources: Vec<ResolvedSource>,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let mut packages = Vec::new();
    for source in &sources {
        let package_root = source.root.clone();
        if !package_root.join("ply-package.toml").exists() {
            return Err(anyhow!(
                "source `{}` is missing ply-package.toml at its root",
                source.id
            ));
        }
        let manifest = load_package_manifest(&package_root)?;
        validate_package_root(&package_root, &manifest)?;
        packages.push(ResolvedPackage {
            source_id: source.id.clone(),
            source_layer: source.layer,
            root: package_root,
            manifest,
        });
    }

    let mut package_names = BTreeMap::new();
    for package in &packages {
        if let Some(existing_source) =
            package_names.insert(package.manifest.name.clone(), package.source_id.clone())
        {
            return Err(anyhow!(
                "duplicate package name `{}` from sources `{}` and `{}`",
                package.manifest.name,
                existing_source,
                package.source_id
            ));
        }
    }

    Ok((sources, packages))
}

fn build_plan(
    project_root: &Path,
    adapter_names: &[String],
    packages: &[ResolvedPackage],
    overlays: &[MergedOverlay],
) -> Result<Vec<PlannedFile>> {
    let mut plan = Vec::new();
    let mut seen = BTreeMap::new();
    let mut sections = Vec::new();
    let mut state = PlanState {
        plan: &mut plan,
        sections: &mut sections,
        seen: &mut seen,
    };

    for adapter_name in adapter_names {
        let adapter = AdapterKind::parse(adapter_name)?;
        for package in packages {
            if !package_targets_adapter(&package.manifest, adapter)? {
                continue;
            }
            for kind in [
                AssetKind::Commands,
                AssetKind::Skills,
                AssetKind::Agents,
                AssetKind::Rules,
                AssetKind::Hooks,
                AssetKind::OutputStyles,
            ] {
                if !adapter.supports(kind) {
                    continue;
                }
                let source_dir = package.root.join(kind.as_str());
                if source_dir.exists() {
                    let origin_detail = format!(
                        "package {} from source {}",
                        package.manifest.name, package.source_id
                    );
                    let context = PlanContext {
                        project_root,
                        adapter,
                        kind,
                        origin_layer: package.source_layer,
                        origin_detail: origin_detail.clone(),
                    };
                    if is_prompt_resource(kind) {
                        collect_prompt_resource_plans(&context, &source_dir, &mut state)?;
                        continue;
                    }
                    match adapter.exposure_mode(kind) {
                        ExposureMode::Direct => {
                            collect_planned_files(&context, &source_dir, &mut state)?
                        }
                        ExposureMode::GeneratedComposite => collect_directory_sections(
                            adapter,
                            kind,
                            &source_dir,
                            package.source_layer,
                            origin_detail,
                            state.sections,
                        )?,
                        ExposureMode::InjectBlock => {}
                    }
                }
            }
            let local_instructions = package.root.join("local-instructions.md");
            if local_instructions.exists() {
                collect_document_section(
                    adapter,
                    AssetKind::LocalInstructions,
                    &local_instructions,
                    package.source_layer,
                    format!(
                        "package {} from source {}",
                        package.manifest.name, package.source_id
                    ),
                    state.sections,
                )?;
            }
        }
    }

    for overlay in overlays {
        let adapter = AdapterKind::parse(&overlay.entry.adapter)?;
        let kind = AssetKind::parse(&overlay.entry.kind)?;
        let source_path = overlay.root.join(&overlay.entry.path);
        if !source_path.exists() {
            continue;
        }
        let origin_detail = format!("overlay {}", overlay.entry.path);
        let context = PlanContext {
            project_root,
            adapter,
            kind,
            origin_layer: overlay.layer,
            origin_detail: origin_detail.clone(),
        };
        if kind.is_directory_based() {
            if is_prompt_resource(kind) {
                collect_prompt_resource_plans(&context, &source_path, &mut state)?;
                continue;
            }
            match adapter.exposure_mode(kind) {
                ExposureMode::Direct => collect_planned_files(&context, &source_path, &mut state)?,
                ExposureMode::GeneratedComposite => collect_directory_sections(
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    origin_detail,
                    state.sections,
                )?,
                ExposureMode::InjectBlock => {}
            }
        } else {
            if is_prompt_resource(kind) {
                collect_prompt_document_plan(&context, &source_path, &mut state)?;
            } else {
                collect_document_section(
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    origin_detail,
                    state.sections,
                )?;
            }
        }
    }

    let direct_plan_snapshot = plan.clone();
    append_managed_file_plans(project_root, &sections, &direct_plan_snapshot, &mut plan)?;

    plan.sort_by(|a, b| a.generated_relative_path.cmp(&b.generated_relative_path));
    Ok(plan)
}

fn append_managed_file_plans(
    project_root: &Path,
    sections: &[CompositeSection],
    existing_plan: &[PlannedFile],
    plan: &mut Vec<PlannedFile>,
) -> Result<()> {
    let claude_sections: Vec<_> = sections
        .iter()
        .filter(|section| section.adapter == AdapterKind::Claude)
        .cloned()
        .collect();
    if !claude_sections.is_empty() {
        let target = AdapterKind::Claude
            .managed_file_path(project_root, AssetKind::LocalInstructions)
            .expect("claude local instructions target");
        let existing = fs::read_to_string(&target).ok();
        let rendered = render_claude_local_instructions(existing.as_deref(), &claude_sections);
        plan.push(PlannedFile {
            adapter: AdapterKind::Claude,
            kind: AssetKind::LocalInstructions,
            exposure_mode: ExposureMode::InjectBlock,
            relative_name: "CLAUDE.local.md".to_string(),
            generated_relative_path: PathBuf::from(".ply")
                .join("generated")
                .join("claude")
                .join("CLAUDE.local.md"),
            exposed_relative_path: PathBuf::from("CLAUDE.local.md"),
            contents: rendered.into_bytes(),
            origin_layer: LayerKind::Project,
            origin_detail: format!("{} ply-managed section(s)", claude_sections.len()),
        });
    }

    let codex_sections: Vec<_> = sections
        .iter()
        .filter(|section| {
            section.adapter == AdapterKind::Codex
                && matches!(
                    section.kind,
                    AssetKind::LocalInstructions | AssetKind::OutputStyles
                )
        })
        .cloned()
        .collect();
    if !codex_sections.is_empty() {
        let rendered = render_codex_override(project_root, &codex_sections)?;
        plan.push(PlannedFile {
            adapter: AdapterKind::Codex,
            kind: AssetKind::LocalInstructions,
            exposure_mode: ExposureMode::GeneratedComposite,
            relative_name: "AGENTS.override.md".to_string(),
            generated_relative_path: PathBuf::from(".ply")
                .join("generated")
                .join("codex")
                .join("AGENTS.override.md"),
            exposed_relative_path: PathBuf::from("AGENTS.override.md"),
            contents: rendered.into_bytes(),
            origin_layer: LayerKind::Project,
            origin_detail: format!("{} ply-managed section(s)", codex_sections.len()),
        });
    }

    let codex_hook_files: Vec<_> = existing_plan
        .iter()
        .filter(|file| file.adapter == AdapterKind::Codex && file.kind == AssetKind::Hooks)
        .cloned()
        .collect();
    if !codex_hook_files.is_empty() {
        let rendered = render_codex_hook_registry(&codex_hook_files);
        plan.push(PlannedFile {
            adapter: AdapterKind::Codex,
            kind: AssetKind::Hooks,
            exposure_mode: ExposureMode::GeneratedComposite,
            relative_name: "hooks.json".to_string(),
            generated_relative_path: PathBuf::from(".ply")
                .join("generated")
                .join("codex")
                .join("hooks.json"),
            exposed_relative_path: PathBuf::from(".codex").join("hooks.json"),
            contents: rendered.into_bytes(),
            origin_layer: LayerKind::Project,
            origin_detail: format!("{} codex hook registration(s)", codex_hook_files.len()),
        });
    }

    Ok(())
}

fn collect_prompt_resource_plans(
    context: &PlanContext<'_>,
    source_dir: &Path,
    state: &mut PlanState<'_>,
) -> Result<()> {
    match context.kind {
        AssetKind::Commands | AssetKind::OutputStyles => {
            for entry in fs::read_dir(source_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() || is_asset_metadata_file(&entry.path()) {
                    continue;
                }
                collect_prompt_document_plan(context, &entry.path(), state)?;
            }
            Ok(())
        }
        AssetKind::Skills | AssetKind::Agents => {
            for entry in fs::read_dir(source_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                collect_prompt_directory_plan(context, &entry.path(), state)?;
            }
            Ok(())
        }
        _ => Err(anyhow!(
            "unsupported prompt resource kind `{}`",
            context.kind.as_str()
        )),
    }
}

fn collect_prompt_document_plan(
    context: &PlanContext<'_>,
    source_file: &Path,
    state: &mut PlanState<'_>,
) -> Result<()> {
    let metadata = load_document_metadata(source_file)?;
    if !resource_targets_adapter(metadata.as_ref(), context.adapter)? {
        return Ok(());
    }
    let logical_name = prompt_logical_name(source_file)?;
    let managed_name = ensure_managed_name(context.kind, &logical_name);
    let markdown = fs::read_to_string(source_file)?;
    let resource = parse_prompt_resource_file(context.kind, &managed_name, source_file, &markdown)?;

    match (context.adapter, context.kind) {
        (AdapterKind::Claude, AssetKind::Commands)
        | (AdapterKind::Claude, AssetKind::OutputStyles) => {
            let rendered = render_claude_markdown(context.kind, &resource)?;
            let rel = source_file.file_name().ok_or_else(|| {
                anyhow!("invalid prompt resource path `{}`", source_file.display())
            })?;
            let rel = managed_relative_path(context.kind, Path::new(rel))?;
            push_rendered_file(
                context,
                state,
                managed_name,
                rel,
                rendered.into_bytes(),
                ExposureMode::Direct,
            )
        }
        (AdapterKind::Codex, AssetKind::Commands) => {
            let rendered = render_codex_prompt_markdown(&resource);
            let rel = source_file.file_name().ok_or_else(|| {
                anyhow!("invalid prompt resource path `{}`", source_file.display())
            })?;
            let rel = managed_relative_path(context.kind, Path::new(rel))?;
            push_rendered_file(
                context,
                state,
                managed_name,
                rel,
                rendered.into_bytes(),
                ExposureMode::Direct,
            )
        }
        (AdapterKind::Codex, AssetKind::OutputStyles) => {
            let rendered = render_codex_prompt_markdown(&resource);
            if rendered.trim().is_empty() {
                return Ok(());
            }
            state.sections.push(CompositeSection {
                adapter: context.adapter,
                kind: context.kind,
                title: source_file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(context.kind.as_str())
                    .to_string(),
                content: rendered,
                origin_layer: context.origin_layer,
                origin_detail: context.origin_detail.clone(),
            });
            Ok(())
        }
        _ => Err(anyhow!(
            "unexpected prompt document mapping for `{}` `{}`",
            context.adapter.as_str(),
            context.kind.as_str()
        )),
    }
}

fn collect_prompt_directory_plan(
    context: &PlanContext<'_>,
    resource_dir: &Path,
    state: &mut PlanState<'_>,
) -> Result<()> {
    let logical_name = resource_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid prompt resource path `{}`", resource_dir.display()))?
        .to_string();
    let managed_name = ensure_managed_name(context.kind, &logical_name);
    let parent = resource_dir.parent().ok_or_else(|| {
        anyhow!(
            "resource directory `{}` has no parent",
            resource_dir.display()
        )
    })?;
    let metadata = load_resource_metadata(parent, &logical_name)?;
    if !resource_targets_adapter(metadata.as_ref(), context.adapter)? {
        return Ok(());
    }
    let primary_name = primary_markdown_name(context.kind)
        .ok_or_else(|| anyhow!("no primary markdown file for `{}`", context.kind.as_str()))?;
    let primary_path = resource_dir.join(primary_name);
    if !primary_path.exists() {
        return Err(anyhow!(
            "{} `{}` is missing {}",
            context.kind.as_str(),
            resource_dir
                .strip_prefix(parent)
                .unwrap_or(resource_dir)
                .display(),
            primary_name
        ));
    }
    if context.kind == AssetKind::Skills && resource_dir.join("agents").join("openai.yaml").exists()
    {
        return Err(anyhow!(
            "skill `{}` must not author `agents/openai.yaml` directly; use Codex frontmatter metadata instead",
            logical_name
        ));
    }

    let markdown = fs::read_to_string(&primary_path)?;
    let resource =
        parse_prompt_resource_file(context.kind, &managed_name, &primary_path, &markdown)?;

    match (context.adapter, context.kind) {
        (AdapterKind::Claude, AssetKind::Skills) | (AdapterKind::Claude, AssetKind::Agents) => {
            let rendered = render_claude_markdown(context.kind, &resource)?;
            push_rendered_file(
                context,
                state,
                managed_name.clone(),
                PathBuf::from(&managed_name).join(primary_name),
                rendered.into_bytes(),
                ExposureMode::Direct,
            )?;
            copy_prompt_directory_companions(
                context,
                state,
                resource_dir,
                &managed_name,
                &primary_path,
            )
        }
        (AdapterKind::Codex, AssetKind::Skills) => {
            let rendered = render_codex_skill_markdown(&resource)?;
            push_rendered_file(
                context,
                state,
                managed_name.clone(),
                PathBuf::from(&managed_name).join(primary_name),
                rendered.into_bytes(),
                ExposureMode::Direct,
            )?;
            copy_prompt_directory_companions(
                context,
                state,
                resource_dir,
                &managed_name,
                &primary_path,
            )?;
            if let Some(sidecar) = render_codex_skill_sidecar(&resource)? {
                push_rendered_file(
                    context,
                    state,
                    managed_name,
                    PathBuf::from(resource.logical_name.as_str())
                        .join("agents")
                        .join("openai.yaml"),
                    sidecar.into_bytes(),
                    ExposureMode::Direct,
                )?;
            }
            Ok(())
        }
        (AdapterKind::Codex, AssetKind::Agents) => {
            let rendered = render_codex_agent(&resource)?;
            let generated_relative_path = PathBuf::from(".ply")
                .join("generated")
                .join("codex")
                .join("agents")
                .join(format!("{managed_name}.toml"));
            let exposed_relative_path = PathBuf::from(".codex")
                .join("agents")
                .join(format!("{managed_name}.toml"));
            if let Some(index) = state.seen.get(&generated_relative_path).copied() {
                let existing = &state.plan[index];
                return Err(anyhow!(
                    "duplicate managed asset target {} from {} and {}",
                    existing.exposed_relative_path.display(),
                    existing.origin_detail,
                    context.origin_detail
                ));
            } else {
                let index = state.plan.len();
                state.seen.insert(generated_relative_path.clone(), index);
                state.plan.push(PlannedFile {
                    adapter: AdapterKind::Codex,
                    kind: AssetKind::Agents,
                    exposure_mode: ExposureMode::GeneratedComposite,
                    relative_name: managed_name,
                    generated_relative_path,
                    exposed_relative_path,
                    contents: rendered.into_bytes(),
                    origin_layer: context.origin_layer,
                    origin_detail: context.origin_detail.clone(),
                });
            }
            Ok(())
        }
        _ => Err(anyhow!(
            "unexpected prompt directory mapping for `{}` `{}`",
            context.adapter.as_str(),
            context.kind.as_str()
        )),
    }
}

fn parse_prompt_resource_file(
    kind: AssetKind,
    managed_name: &str,
    source_path: &Path,
    markdown: &str,
) -> Result<crate::prompt_resources::ParsedPromptResource> {
    parse_prompt_resource(kind, managed_name, markdown).map_err(|err| {
        let err = if has_unquoted_argument_hint(markdown) {
            err.context(
                "hint: quote `argument-hint` values when using bracketed placeholders such as \"[topic] [--flag=value]\"",
            )
        } else {
            err
        };
        err.context(format!("failed to parse {}", source_path.display()))
    })
}

fn has_unquoted_argument_hint(markdown: &str) -> bool {
    if !markdown.starts_with("---\n") {
        return false;
    }
    let rest = &markdown[4..];
    let Some(end) = rest.find("\n---\n") else {
        return false;
    };
    let frontmatter = &rest[..end];
    frontmatter.lines().any(|line| {
        let trimmed = line.trim_start();
        ["argument-hint:", "argument_hint:"].iter().any(|prefix| {
            if let Some(value) = trimmed.strip_prefix(prefix) {
                value.trim_start().starts_with('[')
            } else {
                false
            }
        })
    })
}

fn copy_prompt_directory_companions(
    context: &PlanContext<'_>,
    state: &mut PlanState<'_>,
    resource_dir: &Path,
    logical_name: &str,
    primary_path: &Path,
) -> Result<()> {
    let files = collect_file_paths(resource_dir)?;
    for file in files {
        if file == primary_path || is_asset_metadata_file(&file) {
            continue;
        }
        let rel = file.strip_prefix(resource_dir)?;
        if context.kind == AssetKind::Skills
            && rel == Path::new("agents").join("openai.yaml").as_path()
        {
            continue;
        }
        push_rendered_file(
            context,
            state,
            logical_name.to_string(),
            PathBuf::from(logical_name).join(rel),
            fs::read(&file)?,
            ExposureMode::Direct,
        )?;
    }
    Ok(())
}

fn push_rendered_file(
    context: &PlanContext<'_>,
    state: &mut PlanState<'_>,
    relative_name: String,
    relative_path_within_kind: PathBuf,
    contents: Vec<u8>,
    exposure_mode: ExposureMode,
) -> Result<()> {
    let generated_relative_path = PathBuf::from(".ply")
        .join("generated")
        .join(context.adapter.as_str())
        .join(context.kind.as_str())
        .join(&relative_path_within_kind);
    let exposed_root = context
        .adapter
        .direct_asset_root(context.project_root, context.kind)
        .ok_or_else(|| {
            anyhow!(
                "no direct root for `{}` `{}`",
                context.adapter.as_str(),
                context.kind.as_str()
            )
        })?;
    let exposed_relative_path = exposed_root
        .strip_prefix(context.project_root)?
        .join(&relative_path_within_kind);

    if let Some(index) = state.seen.get(&generated_relative_path).copied() {
        let existing = &state.plan[index];
        return Err(anyhow!(
            "duplicate managed asset target {} from {} and {}",
            existing.exposed_relative_path.display(),
            existing.origin_detail,
            context.origin_detail
        ));
    } else {
        let index = state.plan.len();
        state.seen.insert(generated_relative_path.clone(), index);
        state.plan.push(PlannedFile {
            adapter: context.adapter,
            kind: context.kind,
            exposure_mode,
            relative_name,
            generated_relative_path,
            exposed_relative_path,
            contents,
            origin_layer: context.origin_layer,
            origin_detail: context.origin_detail.clone(),
        });
    }
    Ok(())
}

fn render_codex_prompt_markdown(
    resource: &crate::prompt_resources::ParsedPromptResource,
) -> String {
    let mut sections = Vec::new();
    if resource.shared.argument_hint.is_some()
        && let Some(metadata) = render_codex_command_metadata(resource)
    {
        sections.push(metadata);
    }
    if let Some(preamble) = render_codex_prompt_preamble(resource) {
        sections.push(preamble);
    }
    if !resource.body.is_empty() {
        sections.push(format!("{}\n", resource.body));
    }

    if sections.is_empty() {
        String::new()
    } else {
        sections.concat()
    }
}

fn collect_planned_files(
    context: &PlanContext<'_>,
    source_dir: &Path,
    state: &mut PlanState<'_>,
) -> Result<()> {
    if context.adapter == AdapterKind::Codex && context.kind == AssetKind::Agents {
        return Ok(());
    }
    let mut metadata_cache: BTreeMap<String, Result<Option<AssetMetadata>>> = BTreeMap::new();
    for file in collect_file_paths(source_dir)? {
        let rel = file.strip_prefix(source_dir)?;
        let top_level_name = rel
            .components()
            .next()
            .ok_or_else(|| anyhow!("empty relative path under {}", source_dir.display()))?
            .as_os_str()
            .to_string_lossy()
            .to_string();
        let metadata = metadata_cache
            .entry(top_level_name.clone())
            .or_insert_with(|| load_resource_metadata(source_dir, &top_level_name))
            .as_ref()
            .map_err(|err| anyhow!(err.to_string()))?;
        if !resource_targets_adapter(metadata.as_ref(), context.adapter)? {
            continue;
        }
        let managed_name = ensure_managed_name(context.kind, &top_level_name);
        let managed_rel = managed_relative_path(context.kind, rel)?;

        let generated_relative_path = PathBuf::from(".ply")
            .join("generated")
            .join(context.adapter.as_str())
            .join(context.kind.as_str())
            .join(&managed_rel);
        let exposed_root = context
            .adapter
            .direct_asset_root(context.project_root, context.kind)
            .ok_or_else(|| {
                anyhow!(
                    "no direct root for `{}` `{}`",
                    context.adapter.as_str(),
                    context.kind.as_str()
                )
            })?;
        let exposed_relative_path = exposed_root
            .strip_prefix(context.project_root)?
            .join(&managed_rel);

        if let Some(index) = state.seen.get(&generated_relative_path).copied() {
            let existing = &state.plan[index];
            return Err(anyhow!(
                "duplicate managed asset target {} from {} and {}",
                existing.exposed_relative_path.display(),
                existing.origin_detail,
                context.origin_detail
            ));
        } else {
            let index = state.plan.len();
            state.seen.insert(generated_relative_path.clone(), index);
            state.plan.push(PlannedFile {
                adapter: context.adapter,
                kind: context.kind,
                exposure_mode: ExposureMode::Direct,
                relative_name: managed_name,
                generated_relative_path,
                exposed_relative_path,
                contents: fs::read(&file)?,
                origin_layer: context.origin_layer,
                origin_detail: context.origin_detail.clone(),
            });
        }
    }
    Ok(())
}

fn collect_document_section(
    adapter: AdapterKind,
    kind: AssetKind,
    source_file: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    sections: &mut Vec<CompositeSection>,
) -> Result<()> {
    let metadata = load_document_metadata(source_file)?;
    if !resource_targets_adapter(metadata.as_ref(), adapter)? {
        return Ok(());
    }
    let content = fs::read_to_string(source_file)?;
    if content.trim().is_empty() {
        return Ok(());
    }
    sections.push(CompositeSection {
        adapter,
        kind,
        title: source_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(kind.as_str())
            .to_string(),
        content,
        origin_layer,
        origin_detail,
    });
    Ok(())
}

fn collect_directory_sections(
    adapter: AdapterKind,
    kind: AssetKind,
    source_dir: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    sections: &mut Vec<CompositeSection>,
) -> Result<()> {
    let mut metadata_cache: BTreeMap<String, Result<Option<AssetMetadata>>> = BTreeMap::new();
    for file in collect_file_paths(source_dir)? {
        let rel = file.strip_prefix(source_dir)?;
        let top_level_name = rel
            .components()
            .next()
            .ok_or_else(|| anyhow!("empty relative path under {}", source_dir.display()))?
            .as_os_str()
            .to_string_lossy()
            .to_string();
        let metadata = metadata_cache
            .entry(top_level_name.clone())
            .or_insert_with(|| load_resource_metadata(source_dir, &top_level_name))
            .as_ref()
            .map_err(|err| anyhow!(err.to_string()))?;
        if !resource_targets_adapter(metadata.as_ref(), adapter)? {
            continue;
        }
        let content = fs::read_to_string(&file)?;
        if content.trim().is_empty() {
            continue;
        }
        sections.push(CompositeSection {
            adapter,
            kind,
            title: rel.display().to_string(),
            content,
            origin_layer,
            origin_detail: origin_detail.clone(),
        });
    }
    Ok(())
}

fn render_claude_local_instructions(
    existing: Option<&str>,
    sections: &[CompositeSection],
) -> String {
    let managed = render_managed_block_body("Ply-managed local instructions", sections);
    upsert_managed_block(existing.unwrap_or_default(), &managed)
}

fn render_codex_override(project_root: &Path, sections: &[CompositeSection]) -> Result<String> {
    let repo_owned = project_root.join("AGENTS.md");
    let repo_content = if repo_owned.exists() {
        fs::read_to_string(&repo_owned)?
    } else {
        String::new()
    };
    let local_sections: Vec<_> = sections
        .iter()
        .filter(|section| section.kind == AssetKind::LocalInstructions)
        .cloned()
        .collect();
    let output_style_sections: Vec<_> = sections
        .iter()
        .filter(|section| section.kind == AssetKind::OutputStyles)
        .cloned()
        .collect();
    let mut parts = vec!["<!-- Generated by ply. Do not edit. -->".to_string()];
    if !repo_content.trim().is_empty() {
        parts.push(repo_content.trim_end().to_string());
    }
    if !local_sections.is_empty() {
        parts.push(render_managed_block_body(
            "Ply-managed local instructions",
            &local_sections,
        ));
    }
    if !output_style_sections.is_empty() {
        parts.push(render_managed_block_body(
            "Ply-managed output styles",
            &output_style_sections,
        ));
    }
    Ok(parts.join("\n\n") + "\n")
}

fn render_codex_hook_registry(hook_files: &[PlannedFile]) -> String {
    let entries = hook_files
        .iter()
        .map(|file| {
            format!(
                "    {{\n      \"name\": \"{}\",\n      \"path\": \"{}\"\n    }}",
                file.relative_name,
                file.exposed_relative_path.display()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n  \"hooks\": [\n{entries}\n  ]\n}}\n")
}

fn render_managed_block_body(title: &str, sections: &[CompositeSection]) -> String {
    let mut lines = vec![format!("## {title}")];
    for section in sections {
        lines.push(String::new());
        lines.push(format!(
            "### {}: {} ({})",
            section.kind.as_str(),
            section.origin_detail,
            section.origin_layer.as_str()
        ));
        lines.push(format!("Source: {}", section.title));
        lines.push(String::new());
        lines.push(section.content.trim().to_string());
    }
    lines.join("\n")
}

fn upsert_managed_block(existing: &str, managed_body: &str) -> String {
    let managed_block = format!("{PLY_MANAGED_START}\n{managed_body}\n{PLY_MANAGED_END}");
    if let Some(start) = existing.find(PLY_MANAGED_START)
        && let Some(end) = existing[start..].find(PLY_MANAGED_END)
    {
        let end_index = start + end + PLY_MANAGED_END.len();
        let mut rendered = String::new();
        rendered.push_str(existing[..start].trim_end());
        if !rendered.trim().is_empty() {
            rendered.push_str("\n\n");
        }
        rendered.push_str(&managed_block);
        let suffix = existing[end_index..].trim();
        if !suffix.is_empty() {
            rendered.push_str("\n\n");
            rendered.push_str(suffix);
        }
        return rendered + "\n";
    }
    if existing.trim().is_empty() {
        return managed_block + "\n";
    }
    format!("{}\n\n{}\n", existing.trim_end(), managed_block)
}

fn render_apply_dry_run(
    config: &ComposedConfig,
    sources: &[ResolvedSource],
    packages: &[ResolvedPackage],
    planned_files: &[PlannedFile],
    drifted: &[DriftedFile],
) -> String {
    let mut lines = Vec::new();
    lines.push("Layering:".to_string());
    for source in sources {
        lines.push(ui::list_item(&format!(
            "source {} [{}] from {}",
            source.id,
            source.kind,
            source.layer.as_str()
        )));
    }
    for package in packages {
        lines.push(ui::list_item(&format!(
            "package {} from {} [{}]",
            package.manifest.name,
            package.source_id,
            package.source_layer.as_str()
        )));
    }
    for overlay in &config.overlays {
        lines.push(ui::list_item(&format!(
            "overlay {} [{}]",
            overlay.entry.path,
            overlay.layer.as_str()
        )));
    }

    lines.push("\nPlanned assets:".to_string());
    for file in planned_files {
        lines.push(ui::list_item(&format!(
            "{} <= {} ({})",
            file.exposed_relative_path.display(),
            file.origin_detail,
            file.origin_layer.as_str()
        )));
    }

    if drifted.is_empty() {
        lines.push("\nNo managed content drift detected.".to_string());
    } else {
        lines.push("\nManaged content drift:".to_string());
        for drift in drifted {
            lines.push(ui::list_item(&format!(
                "{} wants content from {}: {}",
                drift.exposed_relative_path.display(),
                drift.origin_layer.as_str(),
                drift.origin_detail
            )));
        }
        lines.push("\nA real apply will ask for consent before overwriting these exposed files unless `--yes` is provided.".to_string());
    }

    lines.join("\n")
}

fn collect_exposed_drifts(
    project_root: &Path,
    planned_files: &[PlannedFile],
    previous_state: &State,
) -> Result<Vec<DriftedFile>> {
    let previous_owned: BTreeSet<PathBuf> = previous_state
        .owned_paths
        .iter()
        .map(|owned| project_root.join(&owned.exposed_path))
        .collect();
    let mut drifted = Vec::new();
    for file in planned_files {
        let path = exposed_abs_path(project_root, file);
        if !path.exists() || !previous_owned.contains(&path) {
            continue;
        }
        let current = fs::read(&path)?;
        if current == file.contents {
            continue;
        }
        let diff = git::diff_contents(
            &current,
            &file.contents,
            &file.exposed_relative_path.to_string_lossy(),
        )?;
        drifted.push(DriftedFile {
            exposed_relative_path: file.exposed_relative_path.clone(),
            origin_layer: file.origin_layer,
            origin_detail: file.origin_detail.clone(),
            diff,
        });
    }
    Ok(drifted)
}

fn resolve_drift_approvals(drifted: &[DriftedFile], yes: bool) -> Result<BTreeMap<PathBuf, bool>> {
    let mut approvals = BTreeMap::new();
    if yes {
        for drift in drifted {
            approvals.insert(drift.exposed_relative_path.clone(), true);
        }
        return Ok(approvals);
    }

    let mut approve_all = false;
    for drift in drifted {
        if approve_all {
            approvals.insert(drift.exposed_relative_path.clone(), true);
            continue;
        }
        let body = format!(
            "Managed file: {}\nDesired content from {}: {}\n\n{}\n\nChoices:\n{}\n{}\n{}",
            drift.exposed_relative_path.display(),
            drift.origin_layer.as_str(),
            drift.origin_detail,
            drift.diff,
            ui::list_item("y = overwrite this file"),
            ui::list_item("n = keep this file unchanged"),
            ui::list_item("a = overwrite this and all remaining drifted files"),
        );
        loop {
            let answer =
                ui::prompt_choice("Managed file drift detected", &body, "Overwrite? [y/n/a]: ")
                    .map_err(|err| anyhow!("failed to read drift confirmation: {err}"))?;
            match answer.as_str() {
                "y" | "yes" => {
                    approvals.insert(drift.exposed_relative_path.clone(), true);
                    break;
                }
                "n" | "no" => {
                    approvals.insert(drift.exposed_relative_path.clone(), false);
                    break;
                }
                "a" => {
                    approvals.insert(drift.exposed_relative_path.clone(), true);
                    approve_all = true;
                    break;
                }
                _ => {}
            }
        }
    }
    Ok(approvals)
}

fn write_generated_tree(project_root: &Path, planned_files: &[PlannedFile]) -> Result<()> {
    let generated_root = project_root.join(".ply").join("generated");
    if generated_root.exists() {
        fs::remove_dir_all(&generated_root)?;
    }
    fs::create_dir_all(&generated_root)?;
    for file in planned_files {
        let path = generated_abs_path(project_root, file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.contents)?;
    }
    Ok(())
}

fn verify_exposed_targets(
    project_root: &Path,
    planned_files: &[PlannedFile],
    previous_state: &State,
) -> Result<()> {
    let previous_owned: BTreeSet<PathBuf> = previous_state
        .owned_paths
        .iter()
        .map(|owned| project_root.join(&owned.exposed_path))
        .collect();
    for file in planned_files {
        let path = exposed_abs_path(project_root, file);
        if git::is_tracked(project_root, &path)? {
            return Err(anyhow!(
                "refusing to overwrite tracked path {}",
                file.exposed_relative_path.display()
            ));
        }
        if path.exists()
            && !previous_owned.contains(&path)
            && file.exposure_mode != ExposureMode::InjectBlock
        {
            return Err(anyhow!(
                "refusing to overwrite unmanaged path {}",
                file.exposed_relative_path.display()
            ));
        }
        if !git::is_ignored(project_root, &path)? {
            return Err(anyhow!(
                "target {} is not ignored by git; run `ply init` or add local excludes",
                file.exposed_relative_path.display()
            ));
        }
    }
    Ok(())
}

fn write_exposed_tree(
    project_root: &Path,
    planned_files: &[PlannedFile],
    approved_paths: &BTreeSet<PathBuf>,
) -> Result<()> {
    for file in planned_files {
        let path = exposed_abs_path(project_root, file);
        if path.exists()
            && !approved_paths.is_empty()
            && !approved_paths.contains(&file.exposed_relative_path)
        {
            let previous = fs::read(&path)?;
            if previous != file.contents {
                continue;
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.contents)?;
    }
    Ok(())
}

fn remove_stale_paths(
    project_root: &Path,
    previous_state: &State,
    planned_files: &[PlannedFile],
) -> Result<()> {
    let desired: BTreeSet<PathBuf> = planned_files
        .iter()
        .map(|file| exposed_abs_path(project_root, file))
        .collect();
    for owned in &previous_state.owned_paths {
        let path = project_root.join(&owned.exposed_path);
        if !desired.contains(&path) && path.exists() {
            fs::remove_file(&path)?;
            prune_empty_parents(project_root, &path)?;
        }
    }
    Ok(())
}

fn prune_empty_parents(project_root: &Path, path: &Path) -> Result<()> {
    let stop_roots = [
        project_root.join(".agents"),
        project_root.join(".claude"),
        project_root.join(".codex"),
        project_root.join(".ply"),
    ];
    let mut current = path.parent();
    while let Some(dir) = current {
        if stop_roots.iter().any(|root| root == dir) || dir == project_root {
            break;
        }
        if fs::read_dir(dir)?.next().is_none() {
            fs::remove_dir(dir)?;
            current = dir.parent();
        } else {
            break;
        }
    }
    Ok(())
}

fn collect_file_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    visit_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn managed_cleanup_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();

    for managed_asset in collect_managed_asset_roots(root)? {
        paths.insert(managed_asset);
    }

    for relative in [
        PathBuf::from("ply.toml"),
        PathBuf::from("ply.lock"),
        PathBuf::from(".ply"),
        PathBuf::from("ply-packages").join("example-review"),
    ] {
        let path = root.join(relative);
        if path.exists() {
            paths.insert(path);
        }
    }

    Ok(paths.into_iter().collect())
}

fn managed_output_cleanup_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = collect_managed_asset_roots(root)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let ply_state = root.join(".ply");
    if ply_state.exists() {
        paths.insert(ply_state);
    }
    Ok(paths.into_iter().collect())
}

fn collect_managed_asset_roots(project_root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for (adapter, kind) in [
        (AdapterKind::Codex, AssetKind::Commands),
        (AdapterKind::Codex, AssetKind::Skills),
        (AdapterKind::Codex, AssetKind::Agents),
        (AdapterKind::Codex, AssetKind::Rules),
        (AdapterKind::Codex, AssetKind::Hooks),
        (AdapterKind::Claude, AssetKind::Commands),
        (AdapterKind::Claude, AssetKind::Skills),
        (AdapterKind::Claude, AssetKind::Agents),
        (AdapterKind::Claude, AssetKind::Rules),
        (AdapterKind::Claude, AssetKind::Hooks),
        (AdapterKind::Claude, AssetKind::OutputStyles),
    ] {
        let Some(root) = adapter.direct_asset_root(project_root, kind) else {
            continue;
        };
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("ply-") {
                paths.push(entry.path());
            }
        }
    }

    for path in [
        project_root.join("AGENTS.override.md"),
        project_root.join("CLAUDE.local.md"),
        project_root.join(".codex").join("hooks.json"),
    ] {
        if path.exists() {
            paths.push(path);
        }
    }

    paths.sort();
    Ok(paths)
}

fn visit_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_files(root, &path, out)?;
        } else if file_type.is_file() {
            if is_asset_metadata_file(&path) {
                continue;
            }
            out.push(path);
        } else {
            return Err(anyhow!(
                "unsupported non-file asset at {}",
                path.strip_prefix(root).unwrap_or(&path).display()
            ));
        }
    }
    Ok(())
}

fn generated_abs_path(project_root: &Path, file: &PlannedFile) -> PathBuf {
    project_root.join(&file.generated_relative_path)
}

fn exposed_abs_path(project_root: &Path, file: &PlannedFile) -> PathBuf {
    project_root.join(&file.exposed_relative_path)
}

fn render_report_sections(sections: &[(&str, Vec<String>)]) -> String {
    let mut rendered = Vec::new();
    for (title, lines) in sections {
        if lines.is_empty() {
            continue;
        }
        rendered.push(format!("{title}:"));
        rendered.extend(lines.iter().cloned());
    }
    rendered.join("\n\n")
}

fn content_digest(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_asset_metadata_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "ply-asset.toml" || name.ends_with(".ply-asset.toml")
}

fn load_resource_metadata(
    source_dir: &Path,
    top_level_name: &str,
) -> Result<Option<AssetMetadata>> {
    let top_level_path = source_dir.join(top_level_name);
    let metadata_path = if top_level_path.is_dir() {
        top_level_path.join("ply-asset.toml")
    } else {
        source_dir.join(format!("{top_level_name}.ply-asset.toml"))
    };
    load_asset_metadata(&metadata_path)
}

fn load_document_metadata(source_file: &Path) -> Result<Option<AssetMetadata>> {
    let file_name = source_file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid source file name at {}", source_file.display()))?;
    let metadata_path = source_file.with_file_name(format!("{file_name}.ply-asset.toml"));
    load_asset_metadata(&metadata_path)
}

fn load_asset_metadata(path: &Path) -> Result<Option<AssetMetadata>> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let metadata: AssetMetadata =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(metadata))
}

fn resource_targets_adapter(
    metadata: Option<&AssetMetadata>,
    adapter: AdapterKind,
) -> Result<bool> {
    let Some(metadata) = metadata else {
        return Ok(true);
    };
    if metadata.targets.is_empty() {
        return Ok(true);
    }
    for target in &metadata.targets {
        AdapterKind::parse(target)?;
    }
    Ok(metadata
        .targets
        .iter()
        .any(|target| target == adapter.as_str()))
}

fn package_targets_adapter(manifest: &PackageManifest, adapter: AdapterKind) -> Result<bool> {
    if manifest.targets.is_empty() {
        return Ok(true);
    }
    for target in &manifest.targets {
        AdapterKind::parse(target)?;
    }
    Ok(manifest
        .targets
        .iter()
        .any(|target| target == adapter.as_str()))
}

fn validate_package_root(package_root: &Path, manifest: &PackageManifest) -> Result<()> {
    let mut managed_asset_roots = 0usize;
    for entry in fs::read_dir(package_root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "ply-package.toml" || is_asset_metadata_file(&path) {
            continue;
        }
        if matches!(
            name.as_str(),
            ".claude" | ".agents" | ".codex" | ".cursor" | ".gemini"
        ) {
            return Err(anyhow!(
                "package `{}` contains unsupported adapter directory `{}`; use portable package asset kinds instead",
                manifest.name,
                name
            ));
        }
        if name == "local-instructions.md" {
            if !entry.file_type()?.is_file() {
                return Err(anyhow!(
                    "package `{}` must provide `local-instructions.md` as a file",
                    manifest.name
                ));
            }
            managed_asset_roots += 1;
            let metadata = load_document_metadata(&path)?;
            validate_package_metadata_targets(manifest, metadata.as_ref(), &path)?;
            continue;
        }

        let Ok(kind) = AssetKind::parse(&name) else {
            continue;
        };
        if !entry.file_type()?.is_dir() {
            return Err(anyhow!(
                "package `{}` asset kind `{}` must be a directory",
                manifest.name,
                kind.as_str()
            ));
        }
        managed_asset_roots += 1;
        validate_asset_kind_layout(&path, kind, manifest)?;
    }

    if managed_asset_roots == 0 {
        return Err(anyhow!(
            "package `{}` does not expose any supported managed assets",
            manifest.name
        ));
    }

    Ok(())
}

fn validate_asset_kind_layout(
    asset_root: &Path,
    kind: AssetKind,
    manifest: &PackageManifest,
) -> Result<()> {
    match kind {
        AssetKind::Commands | AssetKind::OutputStyles => {
            for entry in fs::read_dir(asset_root)? {
                let entry = entry?;
                let path = entry.path();
                if is_asset_metadata_file(&path) {
                    continue;
                }
                if !entry.file_type()?.is_file() {
                    return Err(anyhow!(
                        "package `{}` asset kind `{}` expects files directly under `{}`",
                        manifest.name,
                        kind.as_str(),
                        asset_root.display()
                    ));
                }
                let metadata = load_document_metadata(&path)?;
                validate_package_metadata_targets(manifest, metadata.as_ref(), &path)?;
            }
        }
        AssetKind::Skills | AssetKind::Agents => {
            for entry in fs::read_dir(asset_root)? {
                let entry = entry?;
                let path = entry.path();
                if !entry.file_type()?.is_dir() {
                    return Err(anyhow!(
                        "package `{}` asset kind `{}` expects directories directly under `{}`",
                        manifest.name,
                        kind.as_str(),
                        asset_root.display()
                    ));
                }
                let resource_name = entry.file_name().to_string_lossy().to_string();
                let metadata = load_resource_metadata(asset_root, &resource_name)?;
                validate_package_metadata_targets(manifest, metadata.as_ref(), &path)?;
            }
        }
        AssetKind::Rules | AssetKind::Hooks => {
            let _ = collect_file_paths(asset_root)?;
            let mut metadata_cache: BTreeMap<String, Result<Option<AssetMetadata>>> =
                BTreeMap::new();
            for file in collect_file_paths(asset_root)? {
                let rel = file.strip_prefix(asset_root)?;
                let top_level_name = rel
                    .components()
                    .next()
                    .ok_or_else(|| anyhow!("empty relative path under {}", asset_root.display()))?
                    .as_os_str()
                    .to_string_lossy()
                    .to_string();
                let metadata = metadata_cache
                    .entry(top_level_name.clone())
                    .or_insert_with(|| load_resource_metadata(asset_root, &top_level_name))
                    .as_ref()
                    .map_err(|err| anyhow!(err.to_string()))?;
                validate_package_metadata_targets(manifest, metadata.as_ref(), &file)?;
            }
        }
        AssetKind::LocalInstructions => {}
    }
    Ok(())
}

fn validate_package_metadata_targets(
    manifest: &PackageManifest,
    metadata: Option<&AssetMetadata>,
    resource_path: &Path,
) -> Result<()> {
    let Some(metadata) = metadata else {
        return Ok(());
    };
    for target in &metadata.targets {
        AdapterKind::parse(target)?;
        if !manifest.targets.is_empty() && !manifest.targets.iter().any(|allowed| allowed == target)
        {
            return Err(anyhow!(
                "resource `{}` targets `{}` but package `{}` only allows [{}]",
                resource_path.display(),
                target,
                manifest.name,
                manifest.targets.join(", ")
            ));
        }
    }
    Ok(())
}

fn merge_local_manifest(
    mut manifest: Manifest,
    local_manifest: Option<LocalManifest>,
) -> Result<Manifest> {
    let Some(local_manifest) = local_manifest else {
        return Ok(manifest);
    };

    let mut source_index = BTreeMap::new();
    for (index, source) in manifest.sources.iter().enumerate() {
        source_index.insert(source.id.clone(), index);
    }

    for local_source in local_manifest.sources {
        if let Some(index) = source_index.get(&local_source.id).copied() {
            apply_local_source_override(&mut manifest.sources[index], &local_source);
        } else {
            manifest
                .sources
                .push(materialize_local_source(local_source)?);
        }
    }
    Ok(manifest)
}

fn apply_local_source_override(target: &mut SourceConfig, local: &LocalSourceConfig) {
    if let Some(kind) = &local.kind {
        target.kind = kind.clone();
    }
    if local.path.is_some() {
        target.path = local.path.clone();
    }
    if local.repo.is_some() {
        target.repo = local.repo.clone();
        target.url = None;
    }
    if local.url.is_some() {
        target.url = local.url.clone();
        target.repo = None;
    }
    if local.rev.is_some() {
        target.rev = local.rev.clone();
    }
}

fn materialize_local_source(local: LocalSourceConfig) -> Result<SourceConfig> {
    let kind = local.kind.ok_or_else(|| {
        anyhow!(
            "local source `{}` must define `kind` when adding a new source",
            local.id
        )
    })?;
    Ok(SourceConfig {
        id: local.id,
        kind,
        path: local.path,
        repo: local.repo,
        url: local.url,
        rev: local.rev,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn exec_in(project_root: &Path, args: &[&str]) -> Result<()> {
        let status = Command::new("git")
            .args(args)
            .current_dir(project_root)
            .status()?;
        if !status.success() {
            return Err(anyhow!("git command failed: {:?}", args));
        }
        Ok(())
    }

    fn write(path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn example_package_root(project_root: &Path) -> PathBuf {
        project_root.join("ply-packages").join("example-review")
    }

    fn make_project() -> Result<TempDir> {
        let temp = TempDir::new()?;
        exec_in(temp.path(), &["init"])?;
        exec_in(temp.path(), &["config", "user.email", "test@example.com"])?;
        exec_in(temp.path(), &["config", "user.name", "Test User"])?;
        Ok(temp)
    }

    fn init_test_project(project_root: &Path, scaffold_local_packages: bool) -> Result<()> {
        init_project(
            project_root,
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages,
                    ignore_config: false,
                    adapters: &["codex", "claude"],
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        let mut manifest = load_manifest_for_edit(project_root)?;
        manifest.install.use_global = false;
        config::write_manifest(project_root, &manifest)?;
        Ok(())
    }

    fn init_git_package_repo(path: &Path, package_name: &str) -> Result<String> {
        fs::create_dir_all(path.join("skills").join("review"))?;
        exec_in(path, &["init"])?;
        exec_in(path, &["config", "user.email", "test@example.com"])?;
        exec_in(path, &["config", "user.name", "Test User"])?;
        write(
            &path.join("ply-package.toml"),
            &format!("name = \"{package_name}\"\n"),
        )?;
        write(
            &path.join("skills").join("review").join("SKILL.md"),
            "# review\n",
        )?;
        exec_in(path, &["add", "."])?;
        exec_in(path, &["commit", "-m", "initial package"])?;
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()?;
        if !output.status.success() {
            return Err(anyhow!("git rev-parse failed for {}", path.display()));
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    fn git_head(path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()?;
        if !output.status.success() {
            return Err(anyhow!("git rev-parse failed for {}", path.display()));
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    #[test]
    fn merge_local_manifest_overrides_source_repo() -> Result<()> {
        let manifest = Manifest {
            schema_version: 1,
            install: config::InstallConfig::default(),
            adapters: vec!["codex".to_string()],
            sources: vec![SourceConfig {
                id: "team".to_string(),
                kind: "git".to_string(),
                path: None,
                repo: Some("owner/repo".to_string()),
                url: None,
                rev: Some("main".to_string()),
            }],
        };
        let local = LocalManifest {
            schema_version: 1,
            sources: vec![LocalSourceConfig {
                id: "team".to_string(),
                kind: None,
                path: None,
                repo: Some("git@github.com:owner/repo.git".to_string()),
                url: None,
                rev: Some("feature".to_string()),
            }],
            overlays: Vec::new(),
        };

        let merged = merge_local_manifest(manifest, Some(local))?;
        assert_eq!(
            merged.sources[0].repo.as_deref(),
            Some("git@github.com:owner/repo.git")
        );
        assert_eq!(merged.sources[0].rev.as_deref(), Some("feature"));
        Ok(())
    }

    #[test]
    fn init_scaffolds_project_files() -> Result<()> {
        let temp = make_project()?;
        let report = init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                    adapters: &["codex", "claude"],
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        assert!(report.config_root.join("ply.toml").exists());
        assert!(report.config_root.join("ply.local.toml").exists());
        assert!(
            report
                .worktree_root
                .join(".ply")
                .join("state.json")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn linked_worktree_uses_main_config_and_keeps_output_state_local() -> Result<()> {
        let temp = TempDir::new()?;
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        fs::create_dir_all(&main)?;
        exec_in(&main, &["init", "-b", "main"])?;
        exec_in(&main, &["config", "user.email", "test@example.com"])?;
        exec_in(&main, &["config", "user.name", "Test User"])?;
        write(&main.join("README.md"), "# fixture\n")?;
        exec_in(&main, &["add", "README.md"])?;
        exec_in(&main, &["commit", "-m", "initial"])?;

        init_project(
            &main,
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: true,
                    adapters: &["codex"],
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        let mut manifest = load_manifest_for_edit(&main)?;
        manifest.install.use_global = false;
        config::write_manifest(&main, &manifest)?;

        exec_in(
            &main,
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        )?;

        let context = git::repository_context(&linked)?;
        assert_eq!(context.config_root, main);
        assert_eq!(context.worktree_root, linked);

        let report = apply(
            &linked,
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;
        assert!(report.body.contains("configuration (main worktree)"));
        assert!(linked.join(".ply/state.json").exists());
        assert!(main.join("ply.lock").exists());
        assert!(!linked.join("ply.lock").exists());
        assert!(linked.join(".agents/skills/ply-review-diff").exists());

        let doctor_report = doctor(&linked, CommandTarget::Project)?;
        assert!(doctor_report.contains(&main.display().to_string()));
        assert!(doctor_report.contains(&linked.display().to_string()));

        let cleanup = clean_project(
            &linked,
            CleanOptions {
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        assert_eq!(cleanup.config_root, main);
        assert!(!cleanup.updated_git_excludes);
        assert!(cleanup.config_root.join("ply.toml").exists());
        assert!(!linked.join(".ply").exists());
        Ok(())
    }

    #[test]
    fn apply_copies_assets_from_path_source() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;
        assert!(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn apply_uses_source_root_without_package_selection() -> Result<()> {
        let temp = make_project()?;
        let package_root = temp.path().join("fixture-package");
        fs::create_dir_all(package_root.join("skills").join("check"))?;
        write(
            &package_root.join("ply-package.toml"),
            "name = \"fixture-package\"\n",
        )?;
        write(
            &package_root.join("skills").join("check").join("SKILL.md"),
            "# check\n",
        )?;
        let manifest = format!(
            "schema_version = 1\nadapters = [\"codex\", \"claude\"]\n\n[install]\nuse_global = false\n\n[[sources]]\nid = \"fixture\"\nkind = \"path\"\npath = \"{}\"\n",
            package_root.display()
        );
        write(&temp.path().join("ply.toml"), &manifest)?;
        write(&temp.path().join("ply.local.toml"), "schema_version = 1\n")?;
        write(
            &temp.path().join(".ply").join("state.json"),
            "{\n  \"schema_version\": 1,\n  \"ignore_config\": false,\n  \"owned_paths\": []\n}\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        assert!(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-check")
                .join("SKILL.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn apply_exposes_agents_for_claude_and_codex() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        let agent_dir = package_root.join("agents").join("reviewer");
        fs::create_dir_all(&agent_dir)?;
        write(
            &agent_dir.join("AGENT.md"),
            "# reviewer\n\nPly-managed agent for focused review.\n\nReview carefully.\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let codex_agent = fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("agents")
                .join("ply-reviewer.toml"),
        )?;
        assert!(codex_agent.contains("name = \"ply-reviewer\""));
        assert!(codex_agent.contains("description = \"Ply-managed agent for focused review.\""));
        assert!(codex_agent.contains("developer_instructions = "));

        assert!(
            temp.path()
                .join(".claude")
                .join("agents")
                .join("ply-reviewer")
                .join("AGENT.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn agent_targets_can_limit_codex_generation() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        let agent_dir = package_root.join("agents").join("claude-reviewer");
        fs::create_dir_all(&agent_dir)?;
        write(
            &agent_dir.join("AGENT.md"),
            "# claude-reviewer\n\nClaude-only agent.\n",
        )?;
        write(
            &agent_dir.join("ply-asset.toml"),
            "targets = [\"claude\"]\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        assert!(
            temp.path()
                .join(".claude")
                .join("agents")
                .join("ply-claude-reviewer")
                .join("AGENT.md")
                .exists()
        );
        assert!(
            !temp
                .path()
                .join(".codex")
                .join("agents")
                .join("ply-claude-reviewer.toml")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn resource_metadata_targets_selected_adapters() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(
            &package_root
                .join("skills")
                .join("review-diff")
                .join("ply-asset.toml"),
            "targets = [\"claude\"]\n",
        )?;
        let codex_only = package_root.join("skills").join("codex-only");
        fs::create_dir_all(&codex_only)?;
        write(&codex_only.join("SKILL.md"), "# codex-only\n")?;
        write(
            &codex_only.join("ply-asset.toml"),
            "targets = [\"codex\"]\n",
        )?;
        let shared = package_root.join("skills").join("shared");
        fs::create_dir_all(&shared)?;
        write(&shared.join("SKILL.md"), "# shared\n")?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        assert!(
            !temp
                .path()
                .join(".agents")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-codex-only")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            !temp
                .path()
                .join(".claude")
                .join("skills")
                .join("ply-codex-only")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-shared")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("ply-shared")
                .join("SKILL.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn file_resource_metadata_targets_selected_adapters() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        let commands_dir = package_root.join("commands");
        fs::create_dir_all(&commands_dir)?;
        write(
            &commands_dir.join("codex-only.md"),
            "Run Codex-specific review steps.\n",
        )?;
        write(
            &commands_dir.join("codex-only.md.ply-asset.toml"),
            "targets = [\"codex\"]\n",
        )?;
        write(
            &package_root.join("local-instructions.md"),
            "Shared local instruction.\n",
        )?;
        write(
            &package_root.join("local-instructions.md.ply-asset.toml"),
            "targets = [\"claude\"]\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        assert!(
            temp.path()
                .join(".agents")
                .join("commands")
                .join("ply-codex-only.md")
                .exists()
        );
        assert!(
            !temp
                .path()
                .join(".claude")
                .join("commands")
                .join("ply-codex-only.md")
                .exists()
        );

        assert!(!temp.path().join("AGENTS.override.md").exists());

        let claude_local = fs::read_to_string(temp.path().join("CLAUDE.local.md"))?;
        assert!(claude_local.contains("Shared local instruction."));
        Ok(())
    }

    #[test]
    fn package_targets_can_limit_generation() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(
            &package_root.join("ply-package.toml"),
            "name = \"review-diff\"\ntargets = [\"claude\"]\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        assert!(
            !temp
                .path()
                .join(".agents")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("ply-review-diff")
                .join("SKILL.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn reject_resource_target_outside_package_targets() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(
            &package_root.join("ply-package.toml"),
            "name = \"review-diff\"\ntargets = [\"claude\"]\n",
        )?;
        write(
            &package_root
                .join("skills")
                .join("review-diff")
                .join("ply-asset.toml"),
            "targets = [\"codex\"]\n",
        )?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("only allows [claude]"));
        Ok(())
    }

    #[test]
    fn reject_package_with_unsupported_adapter_directory() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        fs::create_dir_all(package_root.join(".claude"))?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported adapter directory `.claude`")
        );
        Ok(())
    }

    #[test]
    fn reject_duplicate_package_names_across_sources() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let first = temp.path().join("pkg-one");
        let second = temp.path().join("pkg-two");
        fs::create_dir_all(first.join("skills").join("one"))?;
        fs::create_dir_all(second.join("skills").join("two"))?;
        write(&first.join("ply-package.toml"), "name = \"shared\"\n")?;
        write(&second.join("ply-package.toml"), "name = \"shared\"\n")?;
        write(
            &first.join("skills").join("one").join("SKILL.md"),
            "# one\n",
        )?;
        write(
            &second.join("skills").join("two").join("SKILL.md"),
            "# two\n",
        )?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "one".to_string(),
                location: AddSourceLocation::Path("./pkg-one".to_string()),
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;
        add_source(
            temp.path(),
            AddSourceRequest {
                id: "two".to_string(),
                location: AddSourceLocation::Path("./pkg-two".to_string()),
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate package name `shared`"));
        Ok(())
    }

    #[test]
    fn reject_duplicate_managed_asset_targets_across_sources() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let first = temp.path().join("pkg-one");
        let second = temp.path().join("pkg-two");
        fs::create_dir_all(first.join("commands"))?;
        fs::create_dir_all(second.join("commands"))?;
        write(&first.join("ply-package.toml"), "name = \"one\"\n")?;
        write(&second.join("ply-package.toml"), "name = \"two\"\n")?;
        write(&first.join("commands").join("review.md"), "Review one.\n")?;
        write(&second.join("commands").join("review.md"), "Review two.\n")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "one".to_string(),
                location: AddSourceLocation::Path("./pkg-one".to_string()),
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;
        add_source(
            temp.path(),
            AddSourceRequest {
                id: "two".to_string(),
                location: AddSourceLocation::Path("./pkg-two".to_string()),
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate managed asset target"));
        assert!(err.to_string().contains(".agents/commands/ply-review.md"));
        Ok(())
    }

    #[test]
    fn prompt_parse_errors_include_source_path_and_argument_hint_guidance() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(
            &package_root.join("commands").join("broken.md"),
            r#"---
name: broken
description: Broken command
argument-hint: [topic] [--flag=value]
---

Use $ARGUMENTS.
"#,
        )?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("failed to parse"));
        assert!(
            err.to_string().contains(
                &package_root
                    .join("commands")
                    .join("broken.md")
                    .display()
                    .to_string()
            )
        );
        assert!(crate::ui::error_body(&err).contains("quote `argument-hint` values"));
        Ok(())
    }

    #[test]
    fn apply_dry_run_reports_drift() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;
        let skill = temp
            .path()
            .join(".agents")
            .join("skills")
            .join("ply-review-diff")
            .join("SKILL.md");
        write(&skill, "# changed\n")?;
        let report = apply(
            temp.path(),
            ApplyOptions {
                dry_run: true,
                yes: false,
            },
        )?;
        assert!(report.body.contains("Managed content drift"));
        Ok(())
    }

    #[test]
    fn apply_generates_codex_override_and_hook_registry() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(
            &temp.path().join("AGENTS.md"),
            "# Repo instructions\n\nKeep tests passing.\n",
        )?;
        write(
            &package_root.join("local-instructions.md"),
            "Prefer local-first workflows.\n",
        )?;
        write(
            &package_root.join("output-styles").join("ply-review.md"),
            "Be concise and bug-focused.\n",
        )?;
        write(
            &package_root.join("rules").join("ply-safe.md"),
            "Never mutate tracked files without consent.\n",
        )?;
        write(
            &package_root.join("hooks").join("ply-lint.sh"),
            "#!/usr/bin/env bash\necho lint\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let override_file = fs::read_to_string(temp.path().join("AGENTS.override.md"))?;
        assert!(override_file.contains("# Repo instructions"));
        assert!(override_file.contains("Prefer local-first workflows."));
        assert!(override_file.contains("Be concise and bug-focused."));

        let hook_registry = fs::read_to_string(temp.path().join(".codex").join("hooks.json"))?;
        assert!(hook_registry.contains("\"name\": \"ply-lint.sh\""));
        assert!(hook_registry.contains(".codex/hooks/ply-lint.sh"));
        assert!(
            temp.path()
                .join(".codex")
                .join("rules")
                .join("ply-safe.md")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn apply_updates_managed_block_in_claude_local_md() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        write(&temp.path().join("CLAUDE.local.md"), "Personal note.\n")?;
        write(
            &package_root.join("local-instructions.md"),
            "Work through diffs carefully.\n",
        )?;
        write(
            &package_root.join("output-styles").join("ply-review.md"),
            "Surface findings first.\n",
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let local_file = fs::read_to_string(temp.path().join("CLAUDE.local.md"))?;
        assert!(local_file.contains("Personal note."));
        assert!(local_file.contains(PLY_MANAGED_START));
        assert!(local_file.contains("Work through diffs carefully."));
        Ok(())
    }

    #[test]
    fn prompt_frontmatter_renders_adapter_specific_outputs() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());

        write(
            &package_root.join("commands").join("docs.md"),
            r#"---
name: docs-helper
description: Help with project documentation
argument-hint: "[topic]"
claude:
  tools:
    - Read
    - Write
codex:
  model: gpt-5.5
  tools: shell, patch
  reasoning_effort: high
---

Write concise documentation for $ARGUMENTS.
"#,
        )?;

        let skill_root = package_root.join("skills").join("writer");
        fs::create_dir_all(skill_root.join("scripts"))?;
        write(
            &skill_root.join("SKILL.md"),
            r#"---
name: writer
description: Writing workflow
claude:
  tools: Read, Write
codex:
  model: gpt-5.5
  reasoning_effort: medium
  interface:
    display_name: Writer
  policy:
    invocation: manual
  dependencies:
    references:
      - style-guide
---

Write clearly and cite facts.
"#,
        )?;
        write(
            &skill_root.join("scripts").join("helper.sh"),
            "#!/usr/bin/env bash\necho helper\n",
        )?;

        let agent_root = package_root.join("agents").join("reviewer");
        fs::create_dir_all(&agent_root)?;
        write(
            &agent_root.join("AGENT.md"),
            r#"---
name: reviewer
description: Reviews code carefully
claude:
  model: sonnet
  tools: Read, Bash
codex:
  model: gpt-5.5
  reasoning_effort: high
  sandbox_mode: workspace-write
  approval_policy: on-request
---

Review carefully and surface findings first.
"#,
        )?;

        write(
            &package_root.join("output-styles").join("concise.md"),
            r#"---
name: Concise
description: Keep responses tight
keep-coding-instructions: true
codex:
  model: gpt-5.5
  reasoning_effort: low
---

Use short, direct responses.
"#,
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let claude_command = fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("ply-docs.md"),
        )?;
        assert!(claude_command.contains("allowed-tools:"));
        assert!(claude_command.contains("argument-hint:"));
        assert!(claude_command.contains("topic"));

        let codex_command = fs::read_to_string(
            temp.path()
                .join(".agents")
                .join("commands")
                .join("ply-docs.md"),
        )?;
        assert!(codex_command.contains("## Ply Command Metadata"));
        assert!(codex_command.contains("Name: docs-helper"));
        assert!(codex_command.contains("Description: Help with project documentation"));
        assert!(codex_command.contains("Arguments: [topic]"));
        assert!(codex_command.contains("## Ply Codex Settings"));
        assert!(codex_command.contains("Preferred model: gpt-5.5"));
        assert!(codex_command.contains("Write concise documentation"));

        let claude_skill = fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("ply-writer")
                .join("SKILL.md"),
        )?;
        assert!(claude_skill.contains("allowed-tools:"));
        assert!(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-writer")
                .join("scripts")
                .join("helper.sh")
                .exists()
        );
        let codex_skill_sidecar = fs::read_to_string(
            temp.path()
                .join(".agents")
                .join("skills")
                .join("ply-writer")
                .join("agents")
                .join("openai.yaml"),
        )?;
        assert!(codex_skill_sidecar.contains("interface:"));
        assert!(codex_skill_sidecar.contains("policy:"));
        assert!(codex_skill_sidecar.contains("dependencies:"));

        let claude_agent = fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("agents")
                .join("ply-reviewer")
                .join("AGENT.md"),
        )?;
        assert!(claude_agent.contains("model: sonnet"));
        assert!(claude_agent.contains("tools:"));
        let codex_agent = fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("agents")
                .join("ply-reviewer.toml"),
        )?;
        assert!(codex_agent.contains("model = \"gpt-5.5\""));
        assert!(codex_agent.contains("model_reasoning_effort = \"high\""));
        assert!(codex_agent.contains("sandbox_mode = \"workspace-write\""));
        assert!(codex_agent.contains("approval_policy = \"on-request\""));

        let claude_style = fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("output-styles")
                .join("ply-concise.md"),
        )?;
        assert!(claude_style.contains("keep-coding-instructions: true"));
        let codex_override = fs::read_to_string(temp.path().join("AGENTS.override.md"))?;
        assert!(codex_override.contains("## Ply Codex Settings"));
        assert!(codex_override.contains("Use short, direct responses."));
        Ok(())
    }

    #[test]
    fn reject_raw_codex_skill_sidecar_in_package() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        let package_root = example_package_root(temp.path());
        let skill_root = package_root.join("skills").join("bad-sidecar");
        fs::create_dir_all(skill_root.join("agents"))?;
        write(&skill_root.join("SKILL.md"), "# bad-sidecar\n")?;
        write(
            &skill_root.join("agents").join("openai.yaml"),
            "interface:\n  display_name: Bad\n",
        )?;

        let err = apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("must not author `agents/openai.yaml` directly")
        );
        Ok(())
    }

    #[test]
    fn init_package_bootstraps_current_directory() -> Result<()> {
        let temp = TempDir::new()?;
        let report = init_package(
            temp.path(),
            PackageInitRequest {
                name: "review-tools".to_string(),
                path: PathBuf::from("."),
                kinds: vec![AssetKind::Skills, AssetKind::Commands],
                dry_run: false,
            },
        )?;
        assert_eq!(report.target_root, temp.path());
        assert!(temp.path().join("ply-package.toml").exists());
        assert!(temp.path().join("skills").exists());
        assert!(temp.path().join("commands").exists());
        Ok(())
    }

    #[test]
    fn init_package_allows_git_bootstrap_files() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join(".git"))?;
        write(&temp.path().join("README.md"), "# package\n")?;
        let report = init_package(
            temp.path(),
            PackageInitRequest {
                name: "review-tools".to_string(),
                path: PathBuf::from("."),
                kinds: vec![],
                dry_run: false,
            },
        )?;
        assert_eq!(report.target_root, temp.path());
        assert!(temp.path().join("ply-package.toml").exists());
        Ok(())
    }

    #[test]
    fn init_package_can_scaffold_agents_directory() -> Result<()> {
        let temp = TempDir::new()?;
        init_package(
            temp.path(),
            PackageInitRequest {
                name: "review-tools".to_string(),
                path: PathBuf::from("."),
                kinds: vec![AssetKind::Agents],
                dry_run: false,
            },
        )?;
        assert!(temp.path().join("agents").exists());
        Ok(())
    }

    #[test]
    fn init_package_accepts_explicit_target_path() -> Result<()> {
        let temp = TempDir::new()?;
        let target = temp.path().join("review-tools");
        let report = init_package(
            temp.path(),
            PackageInitRequest {
                name: "review-tools".to_string(),
                path: target.clone(),
                kinds: vec![AssetKind::Skills],
                dry_run: false,
            },
        )?;
        assert_eq!(report.target_root, target);
        assert!(
            temp.path()
                .join("review-tools")
                .join("ply-package.toml")
                .exists()
        );
        assert!(temp.path().join("review-tools").join("skills").exists());
        Ok(())
    }

    #[test]
    fn init_package_refuses_unrelated_existing_content() -> Result<()> {
        let temp = TempDir::new()?;
        write(&temp.path().join("notes.txt"), "hello\n")?;
        let err = init_package(
            temp.path(),
            PackageInitRequest {
                name: "review-tools".to_string(),
                path: PathBuf::from("."),
                kinds: vec![],
                dry_run: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("refusing to initialize package"));
        Ok(())
    }

    #[test]
    fn add_path_source_rewrites_manifest() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let package_root = temp.path().join("custom-package");
        fs::create_dir_all(package_root.join("skills").join("check"))?;
        write(
            &package_root.join("ply-package.toml"),
            "name = \"custom-package\"\n",
        )?;
        write(
            &package_root.join("skills").join("check").join("SKILL.md"),
            "# check\n",
        )?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "local".to_string(),
                location: AddSourceLocation::Path("./custom-package".to_string()),
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        let manifest = fs::read_to_string(temp.path().join("ply.toml"))?;
        assert!(manifest.contains("[[sources]]"));
        assert!(manifest.contains("id = \"local\""));
        assert!(manifest.contains("path = \"./custom-package\""));
        Ok(())
    }

    #[test]
    fn add_git_source_updates_lockfile_and_remove_prunes_it() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let repo_root = temp.path().join("team-source");
        let initial_commit = init_git_package_repo(&repo_root, "team-source")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-source".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        let manifest = fs::read_to_string(temp.path().join("ply.toml"))?;
        assert!(manifest.contains("repo = \"./team-source\""));
        let lockfile = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert!(lockfile.contains("id = \"team\""));
        assert!(lockfile.contains("repo = \"./team-source\""));
        assert!(lockfile.contains(&format!("resolved = \"{initial_commit}\"")));

        remove_source(
            temp.path(),
            RemoveSourceRequest {
                id: "team".to_string(),
                force: false,
            },
        )?;

        let manifest = fs::read_to_string(temp.path().join("ply.toml"))?;
        assert!(!manifest.contains("id = \"team\""));
        let lockfile = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert!(!lockfile.contains("id = \"team\""));
        Ok(())
    }

    #[test]
    fn add_git_source_with_ssh_key_writes_ssh_config() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let repo_root = temp.path().join("team-source");
        init_git_package_repo(&repo_root, "team-source")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-source".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::KeyPath("~/.ssh/id_team".to_string()),
            },
            CommandTarget::Project,
        )?;

        let ssh_config = fs::read_to_string(temp.path().join("ply.ssh.toml"))?;
        assert!(ssh_config.contains("[sources.team]"));
        assert!(ssh_config.contains("use_ssh = true"));
        assert!(ssh_config.contains("ssh_key_path = \"~/.ssh/id_team\""));
        Ok(())
    }

    #[test]
    fn remove_source_prunes_ssh_config_and_deletes_empty_file() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;
        let repo_root = temp.path().join("team-source");
        init_git_package_repo(&repo_root, "team-source")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-source".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::DefaultKey,
            },
            CommandTarget::Project,
        )?;
        assert!(temp.path().join("ply.ssh.toml").exists());

        remove_source(
            temp.path(),
            RemoveSourceRequest {
                id: "team".to_string(),
                force: false,
            },
        )?;

        assert!(!temp.path().join("ply.ssh.toml").exists());
        Ok(())
    }

    #[test]
    fn failed_git_add_leaves_manifest_and_ssh_config_unchanged() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;

        let manifest_before = fs::read_to_string(temp.path().join("ply.toml"))?;
        let ssh_exists_before = temp.path().join("ply.ssh.toml").exists();
        let lock_exists_before = temp.path().join("ply.lock").exists();

        let repo_root = temp.path().join("team-source");
        init_git_package_repo(&repo_root, "team-source")?;

        let err = add_source(
            temp.path(),
            AddSourceRequest {
                id: "team".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-source".to_string(),
                    rev: Some("missing-branch".to_string()),
                },
                ssh: AddSourceSshMode::KeyPath("~/.ssh/id_team".to_string()),
            },
            CommandTarget::Project,
        )
        .unwrap_err();
        assert!(!err.to_string().trim().is_empty());

        let manifest_after = fs::read_to_string(temp.path().join("ply.toml"))?;
        assert_eq!(manifest_before, manifest_after);
        assert_eq!(ssh_exists_before, temp.path().join("ply.ssh.toml").exists());
        assert_eq!(lock_exists_before, temp.path().join("ply.lock").exists());
        Ok(())
    }

    #[test]
    fn update_named_git_source_preserves_other_locked_git_revisions() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;

        let repo_one = temp.path().join("team-one");
        let repo_two = temp.path().join("team-two");
        let _first_one = init_git_package_repo(&repo_one, "team-one")?;
        let first_two = init_git_package_repo(&repo_two, "team-two")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team-one".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-one".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;
        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team-two".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-two".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        write(&repo_one.join("README.md"), "# updated\n")?;
        exec_in(&repo_one, &["add", "README.md"])?;
        exec_in(&repo_one, &["commit", "-m", "update package"])?;
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_one)
            .output()?;
        let second_one = String::from_utf8(output.stdout)?.trim().to_string();

        update_sources(
            temp.path(),
            UpdateSourcesRequest {
                source_id: Some("team-one".to_string()),
            },
            CommandTarget::Project,
        )?;

        let lockfile = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert!(lockfile.contains(&format!("resolved = \"{second_one}\"")));
        assert!(lockfile.contains(&format!("resolved = \"{first_two}\"")));
        Ok(())
    }

    #[test]
    fn apply_reuses_locked_git_revision_until_update() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), false)?;

        let repo_root = temp.path().join("team-source");
        let first_commit = init_git_package_repo(&repo_root, "team-source")?;

        add_source(
            temp.path(),
            AddSourceRequest {
                id: "team".to_string(),
                location: AddSourceLocation::Git {
                    repo: "./team-source".to_string(),
                    rev: Some("HEAD".to_string()),
                },
                ssh: AddSourceSshMode::None,
            },
            CommandTarget::Project,
        )?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let exposed_skill = temp
            .path()
            .join(".agents")
            .join("skills")
            .join("ply-review")
            .join("SKILL.md");
        let first_contents = fs::read_to_string(&exposed_skill)?;
        let lock_before = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert!(lock_before.contains(&format!("resolved = \"{first_commit}\"")));

        write(
            &repo_root.join("skills").join("review").join("SKILL.md"),
            "# review\n\nUpdated content.\n",
        )?;
        exec_in(&repo_root, &["add", "skills/review/SKILL.md"])?;
        exec_in(&repo_root, &["commit", "-m", "update package"])?;
        let second_commit = git_head(&repo_root)?;

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let lock_after_apply = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert_eq!(lock_before, lock_after_apply);
        assert_eq!(fs::read_to_string(&exposed_skill)?, first_contents);

        update_sources(
            temp.path(),
            UpdateSourcesRequest { source_id: None },
            CommandTarget::Project,
        )?;
        let lock_after_update = fs::read_to_string(temp.path().join("ply.lock"))?;
        assert!(lock_after_update.contains(&format!("resolved = \"{second_commit}\"")));

        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;

        let updated_contents = fs::read_to_string(&exposed_skill)?;
        assert!(updated_contents.contains("Updated content."));
        Ok(())
    }

    #[test]
    fn diff_groups_exposed_changes() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        apply(
            temp.path(),
            ApplyOptions {
                dry_run: false,
                yes: true,
            },
        )?;
        let skill = temp
            .path()
            .join(".agents")
            .join("skills")
            .join("ply-review-diff")
            .join("SKILL.md");
        write(&skill, "# changed\n")?;

        let report = diff(temp.path())?;
        assert!(report.contains("Exposed changes:"));
        assert!(report.contains("drift .agents/skills/ply-review-diff/SKILL.md"));
        Ok(())
    }

    #[test]
    fn doctor_warns_when_git_excludes_are_missing() -> Result<()> {
        let temp = make_project()?;
        init_test_project(temp.path(), true)?;
        crate::git::remove_local_excludes(temp.path())?;

        let report = doctor(temp.path(), CommandTarget::Project)?;
        assert!(report.contains("Warnings:"));
        assert!(report.contains("missing Ply block in .git/info/exclude"));
        Ok(())
    }

    #[test]
    fn doctor_package_reports_missing_manifest_fix() -> Result<()> {
        let temp = TempDir::new()?;
        let package_root = temp.path().join("review-tools");
        fs::create_dir_all(&package_root)?;

        let err = doctor_package(
            temp.path(),
            PackageDoctorRequest {
                path: package_root.clone(),
                fix: false,
            },
        )
        .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("failed to load"));
        assert!(message.contains("doctor package --fix"));
        Ok(())
    }

    #[test]
    fn get_and_set_package_metadata_round_trip() -> Result<()> {
        let temp = TempDir::new()?;
        let package_root = temp.path().join("review-tools");
        fs::create_dir_all(package_root.join("skills"))?;
        config::write_package_manifest(&package_root, "review-tools")?;
        write(&package_root.join("skills").join("SKILL.md"), "# ignored\n")?;

        let updated = set_package_metadata(
            temp.path(),
            PackageSetRequest {
                path: package_root.clone(),
                key: "license".to_string(),
                value: "MIT".to_string(),
            },
        )?;
        assert!(updated.contains("license=MIT"));

        let license = get_package_metadata(
            temp.path(),
            PackageGetRequest {
                path: package_root.clone(),
                key: Some("license".to_string()),
            },
        )?;
        assert_eq!(license, "MIT");

        let all = get_package_metadata(
            temp.path(),
            PackageGetRequest {
                path: package_root,
                key: None,
            },
        )?;
        assert!(all.contains("name=review-tools"));
        assert!(all.contains("license=MIT"));
        Ok(())
    }
}
