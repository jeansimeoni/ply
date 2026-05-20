use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::adapters::{AdapterKind, AssetKind, ExposureMode};
use crate::config::{
    self, InitOptions, LocalManifest, LocalOverlayConfig, LocalSourceConfig, LockedSource,
    Lockfile, Manifest, OverlayEntry, OwnedPath, PackageManifest, SourceConfig, SshConfigFile,
    SshSourceConfig, State, load_local_manifest_if_present, load_local_overlays, load_manifest,
    load_manifest_if_present, load_package_manifest, load_ssh_config_if_present, load_state,
};
use crate::git;
use crate::prompt_resources::{
    is_prompt_resource, parse_prompt_resource, primary_markdown_name, prompt_logical_name,
    render_claude_markdown, render_codex_agent, render_codex_prompt_preamble,
    render_codex_skill_sidecar,
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
}

#[derive(Debug, Clone)]
pub struct CleanupReport {
    pub removed_items: Vec<String>,
    pub updated_git_excludes: bool,
}

#[derive(Debug, Clone)]
pub struct InitReport {
    pub created_manifest: bool,
    pub created_local_fixture: bool,
    pub ignore_config: bool,
    pub target_root: PathBuf,
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
pub struct ApplyReport {
    pub body: String,
    pub dry_run: bool,
}

pub fn init_project(project_root: &Path, request: InitRequest) -> Result<InitReport> {
    let target_root = target_root(project_root, request.target)?;

    if matches!(request.target, CommandTarget::Project) {
        git::ensure_git_repo(&target_root)?;
    }

    let created_manifest = !target_root.join("ply.toml").exists();
    let created_local_fixture = !target_root
        .join("ply-packages")
        .join("example-review")
        .exists()
        && request.options.scaffold_local_packages;

    if !request.dry_run {
        fs::create_dir_all(target_root.join(".ply").join("generated"))?;
        fs::create_dir_all(
            target_root
                .join(".ply")
                .join("overlays")
                .join("codex")
                .join("skills"),
        )?;
        fs::create_dir_all(
            target_root
                .join(".ply")
                .join("overlays")
                .join("claude")
                .join("skills"),
        )?;
        git::ensure_local_excludes(&target_root, request.options)?;
        config::write_default_manifest(&target_root, request.options)?;
        config::write_default_local_manifest(&target_root)?;
        config::write_state(
            &target_root,
            &State {
                schema_version: 1,
                install_mode: "copy".to_string(),
                ignore_config: request.options.ignore_config,
                owned_paths: Vec::new(),
            },
        )?;
        if request.options.scaffold_local_packages {
            config::write_default_package_fixture(&target_root)?;
        }
    }

    Ok(InitReport {
        created_manifest,
        created_local_fixture,
        ignore_config: request.options.ignore_config,
        target_root,
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

fn ensure_package_bootstrap_target(target_root: &Path, kinds: &[AssetKind]) -> Result<()> {
    if !target_root.exists() {
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(target_root)? {
        entries.push(entry?);
    }

    let allowed_names = [".git", ".gitignore", "README", "README.md", "LICENSE", "LICENSE.md"];
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
    let root = target_root(project_root, options.target)?;
    if matches!(options.target, CommandTarget::Project) {
        git::ensure_git_repo(&root)?;
    }

    let mut items = Vec::new();
    for path in managed_cleanup_paths(&root)? {
        items.push(path.strip_prefix(&root)?.display().to_string());
    }
    let updates_git_excludes = git::has_ply_excludes(&root);

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
    })
}

pub fn clean_project(project_root: &Path, options: CleanOptions) -> Result<CleanupReport> {
    let root = target_root(project_root, options.target)?;
    let preview = preview_cleanup(project_root, options)?;
    if options.dry_run {
        return Ok(CleanupReport {
            removed_items: preview.items,
            updated_git_excludes: preview.updates_git_excludes,
        });
    }

    let mut removed_items = Vec::new();
    for path in managed_cleanup_paths(&root)? {
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
    })
}

pub fn apply(project_root: &Path, options: ApplyOptions) -> Result<ApplyReport> {
    git::ensure_git_repo(project_root)?;
    let previous_state = load_state(project_root)?;
    git::ensure_local_excludes(
        project_root,
        InitOptions {
            scaffold_local_packages: false,
            ignore_config: previous_state.ignore_config,
        },
    )?;
    let composition = compose_project_config(project_root)?;
    let (sources, packages) = resolve_composed(&composition)?;
    let planned_files = build_plan(
        project_root,
        &composition.adapters,
        &packages,
        &composition.overlays,
    )?;
    verify_exposed_targets(project_root, &planned_files, &previous_state)?;
    let drifted = collect_exposed_drifts(project_root, &planned_files, &previous_state)?;

    if options.dry_run {
        let body =
            render_apply_dry_run(&composition, &sources, &packages, &planned_files, &drifted);
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

    write_generated_tree(project_root, &planned_files)?;
    remove_stale_paths(project_root, &previous_state, &planned_files)?;
    write_exposed_tree(project_root, &planned_files, &approved_paths)?;
    let lockfile = Lockfile {
        schema_version: 1,
        sources: sources
            .iter()
            .map(|source| LockedSource {
                id: source.id.clone(),
                kind: source.kind.clone(),
                resolved: source.resolved.clone(),
            })
            .collect(),
    };
    config::write_lockfile(project_root, &lockfile)?;
    let state = State {
        schema_version: 1,
        install_mode: "copy".to_string(),
        ignore_config: previous_state.ignore_config,
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
    config::write_state(project_root, &state)?;

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
        body,
        dry_run: false,
    })
}

pub fn diff(project_root: &Path) -> Result<String> {
    let composition = compose_project_config(project_root)?;
    let (_, packages) = resolve_composed(&composition)?;
    let previous_state = load_state(project_root)?;
    let planned_files = build_plan(
        project_root,
        &composition.adapters,
        &packages,
        &composition.overlays,
    )?;

    let desired_generated: BTreeSet<PathBuf> = planned_files
        .iter()
        .map(|file| generated_abs_path(project_root, file))
        .collect();
    let desired_exposed: BTreeSet<PathBuf> = planned_files
        .iter()
        .map(|file| exposed_abs_path(project_root, file))
        .collect();
    let owned_previous: BTreeSet<PathBuf> = previous_state
        .owned_paths
        .iter()
        .map(|owned| project_root.join(&owned.exposed_path))
        .collect();

    let mut generated_changes = Vec::new();
    let mut exposed_changes = Vec::new();
    let mut stale_paths = Vec::new();
    let mut safety_violations = Vec::new();
    for file in &planned_files {
        let generated = generated_abs_path(project_root, file);
        let exposed = exposed_abs_path(project_root, file);
        if !generated.exists() {
            generated_changes.push(ui::status_line(
                Tone::Info,
                &format!("generate {}", file.generated_relative_path.display()),
            ));
        }
        if git::is_tracked(project_root, &exposed)? {
            safety_violations.push(ui::status_line(
                Tone::Warning,
                &format!("tracked target {}", file.exposed_relative_path.display()),
            ));
        }
        if !git::is_ignored(project_root, &exposed)? {
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
            &format!("remove {}", stale.strip_prefix(project_root)?.display()),
        ));
    }
    for generated_path in collect_file_paths(&project_root.join(".ply").join("generated"))? {
        if !desired_generated.contains(&generated_path) {
            stale_paths.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "remove {}",
                    generated_path.strip_prefix(project_root)?.display()
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
        return Ok("no differences".to_string());
    }
    Ok(rendered)
}

pub fn doctor(project_root: &Path, target: CommandTarget) -> Result<String> {
    let root = target_root(project_root, target)?;
    let composition = if matches!(target, CommandTarget::Global) {
        compose_single_root(&root, LayerKind::Global)?
    } else {
        compose_project_config(project_root)?
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
                &format!("state points to missing generated path {}", owned.generated_path),
            ));
        }
        let exposed = root.join(&owned.exposed_path);
        if !exposed.exists() {
            warnings.push(ui::status_line(
                Tone::Warning,
                &format!("state points to missing exposed path {}", owned.exposed_path),
            ));
        }
    }
    Ok(render_report_sections(&[
        ("Healthy checks", healthy),
        ("Warnings", warnings),
    ]))
}

pub fn list_packages(project_root: &Path, target: CommandTarget) -> Result<String> {
    let root = target_root(project_root, target)?;
    let composition = if matches!(target, CommandTarget::Global) {
        compose_single_root(&root, LayerKind::Global)?
    } else {
        compose_project_config(project_root)?
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
    Ok(lines.join("\n"))
}

pub fn list_sources(project_root: &Path, target: CommandTarget) -> Result<String> {
    let root = target_root(project_root, target)?;
    let composition = if matches!(target, CommandTarget::Global) {
        compose_single_root(&root, LayerKind::Global)?
    } else {
        compose_project_config(project_root)?
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
    Ok(lines.join("\n"))
}

fn target_root(project_root: &Path, target: CommandTarget) -> Result<PathBuf> {
    match target {
        CommandTarget::Project => Ok(project_root.to_path_buf()),
        CommandTarget::Global => config::global_root(),
    }
}

fn compose_project_config(project_root: &Path) -> Result<ComposedConfig> {
    let project_manifest = load_manifest(project_root)?;
    let project_local_manifest = load_local_manifest_if_present(project_root)?;
    let project_ssh_config = load_ssh_config_if_present(project_root)?.unwrap_or_default();
    let project_overlays = load_local_overlays(project_root)?;
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
        root: project_root.to_path_buf(),
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

fn resolve_composed(
    config: &ComposedConfig,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let mut sources = Vec::new();
    for source in &config.sources {
        let resolved = match source.config.kind.as_str() {
            "path" => {
                let path =
                    source
                        .root
                        .join(source.config.path.as_deref().ok_or_else(|| {
                            anyhow!("path source `{}` missing path", source.config.id)
                        })?);
                let root = path.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve path source `{}` at {}",
                        source.config.id,
                        path.display()
                    )
                })?;
                ResolvedSource {
                    id: source.config.id.clone(),
                    kind: source.config.kind.clone(),
                    resolved: root.display().to_string(),
                    root,
                    layer: source.layer,
                }
            }
            "git" => {
                let (root, revision) =
                    git::clone_or_update_source(&source.root, &source.config, source.ssh_config.as_ref())?;
                ResolvedSource {
                    id: source.config.id.clone(),
                    kind: source.config.kind.clone(),
                    resolved: revision,
                    root,
                    layer: source.layer,
                }
            }
            other => return Err(anyhow!("unsupported source kind `{other}`")),
        };
        sources.push(resolved);
    }

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
        packages.push(ResolvedPackage {
            source_id: source.id.clone(),
            source_layer: source.layer,
            root: package_root,
            manifest,
        });
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

    for adapter_name in adapter_names {
        let adapter = AdapterKind::parse(adapter_name)?;
        for package in packages {
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
                    if is_prompt_resource(kind) {
                        collect_prompt_resource_plans(
                            project_root,
                            adapter,
                            kind,
                            &source_dir,
                            package.source_layer,
                            format!(
                                "package {} from source {}",
                                package.manifest.name, package.source_id
                            ),
                            &mut plan,
                            &mut sections,
                            &mut seen,
                        )?;
                        continue;
                    }
                    match adapter.exposure_mode(kind) {
                        ExposureMode::Direct => {
                            let origin_detail = format!(
                                "package {} from source {}",
                                package.manifest.name, package.source_id
                            );
                            collect_planned_files(
                                project_root,
                                adapter,
                                kind,
                                &source_dir,
                                package.source_layer,
                                origin_detail.clone(),
                                &mut plan,
                                &mut seen,
                            )?;
                        }
                        ExposureMode::GeneratedComposite => collect_directory_sections(
                            adapter,
                            kind,
                            &source_dir,
                            package.source_layer,
                            format!(
                                "package {} from source {}",
                                package.manifest.name, package.source_id
                            ),
                            &mut sections,
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
                    format!("package {} from source {}", package.manifest.name, package.source_id),
                    &mut sections,
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
        if kind.is_directory_based() {
            if is_prompt_resource(kind) {
                collect_prompt_resource_plans(
                    project_root,
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    format!("overlay {}", overlay.entry.path),
                    &mut plan,
                    &mut sections,
                    &mut seen,
                )?;
                continue;
            }
            match adapter.exposure_mode(kind) {
                ExposureMode::Direct => collect_planned_files(
                    project_root,
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    format!("overlay {}", overlay.entry.path),
                    &mut plan,
                    &mut seen,
                )?,
                ExposureMode::GeneratedComposite => collect_directory_sections(
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    format!("overlay {}", overlay.entry.path),
                    &mut sections,
                )?,
                ExposureMode::InjectBlock => {}
            }
        } else {
            if is_prompt_resource(kind) {
                collect_prompt_document_plan(
                    project_root,
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    format!("overlay {}", overlay.entry.path),
                    &mut plan,
                    &mut sections,
                    &mut seen,
                )?;
            } else {
                collect_document_section(
                    adapter,
                    kind,
                    &source_path,
                    overlay.layer,
                    format!("overlay {}", overlay.entry.path),
                    &mut sections,
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
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    source_dir: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    sections: &mut Vec<CompositeSection>,
    seen: &mut BTreeMap<PathBuf, usize>,
) -> Result<()> {
    match kind {
        AssetKind::Commands | AssetKind::OutputStyles => {
            for entry in fs::read_dir(source_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() || is_asset_metadata_file(&entry.path()) {
                    continue;
                }
                collect_prompt_document_plan(
                    project_root,
                    adapter,
                    kind,
                    &entry.path(),
                    origin_layer,
                    origin_detail.clone(),
                    plan,
                    sections,
                    seen,
                )?;
            }
            Ok(())
        }
        AssetKind::Skills | AssetKind::Agents => {
            for entry in fs::read_dir(source_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                collect_prompt_directory_plan(
                    project_root,
                    adapter,
                    kind,
                    &entry.path(),
                    origin_layer,
                    origin_detail.clone(),
                    plan,
                    sections,
                    seen,
                )?;
            }
            Ok(())
        }
        _ => Err(anyhow!(
            "unsupported prompt resource kind `{}`",
            kind.as_str()
        )),
    }
}

fn collect_prompt_document_plan(
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    source_file: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    sections: &mut Vec<CompositeSection>,
    seen: &mut BTreeMap<PathBuf, usize>,
) -> Result<()> {
    let metadata = load_document_metadata(source_file)?;
    if !resource_targets_adapter(metadata.as_ref(), adapter)? {
        return Ok(());
    }
    let logical_name = prompt_logical_name(source_file)?;
    let managed_name = ensure_managed_name(kind, &logical_name);
    let markdown = fs::read_to_string(source_file)?;
    let resource = parse_prompt_resource(kind, &managed_name, &markdown)?;

    match (adapter, kind) {
        (AdapterKind::Claude, AssetKind::Commands)
        | (AdapterKind::Claude, AssetKind::OutputStyles) => {
            let rendered = render_claude_markdown(kind, &resource)?;
            let rel = source_file
                .file_name()
                .ok_or_else(|| anyhow!("invalid prompt resource path `{}`", source_file.display()))?;
            let rel = managed_relative_path(kind, Path::new(rel))?;
            push_rendered_file(
                project_root,
                adapter,
                kind,
                managed_name,
                rel,
                rendered.into_bytes(),
                origin_layer,
                origin_detail,
                plan,
                seen,
                ExposureMode::Direct,
            )
        }
        (AdapterKind::Codex, AssetKind::Commands) => {
            let rendered = render_codex_prompt_markdown(&resource);
            let rel = source_file
                .file_name()
                .ok_or_else(|| anyhow!("invalid prompt resource path `{}`", source_file.display()))?;
            let rel = managed_relative_path(kind, Path::new(rel))?;
            push_rendered_file(
                project_root,
                adapter,
                kind,
                managed_name,
                rel,
                rendered.into_bytes(),
                origin_layer,
                origin_detail,
                plan,
                seen,
                ExposureMode::Direct,
            )
        }
        (AdapterKind::Codex, AssetKind::OutputStyles) => {
            let rendered = render_codex_prompt_markdown(&resource);
            if rendered.trim().is_empty() {
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
                content: rendered,
                origin_layer,
                origin_detail,
            });
            Ok(())
        }
        _ => Err(anyhow!(
            "unexpected prompt document mapping for `{}` `{}`",
            adapter.as_str(),
            kind.as_str()
        )),
    }
}

fn collect_prompt_directory_plan(
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    resource_dir: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    _sections: &mut Vec<CompositeSection>,
    seen: &mut BTreeMap<PathBuf, usize>,
) -> Result<()> {
    let logical_name = resource_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid prompt resource path `{}`", resource_dir.display()))?
        .to_string();
    let managed_name = ensure_managed_name(kind, &logical_name);
    let parent = resource_dir
        .parent()
        .ok_or_else(|| anyhow!("resource directory `{}` has no parent", resource_dir.display()))?;
    let metadata = load_resource_metadata(parent, &logical_name)?;
    if !resource_targets_adapter(metadata.as_ref(), adapter)? {
        return Ok(());
    }
    let primary_name = primary_markdown_name(kind)
        .ok_or_else(|| anyhow!("no primary markdown file for `{}`", kind.as_str()))?;
    let primary_path = resource_dir.join(primary_name);
    if !primary_path.exists() {
        return Err(anyhow!(
            "{} `{}` is missing {}",
            kind.as_str(),
            resource_dir.strip_prefix(parent).unwrap_or(resource_dir).display(),
            primary_name
        ));
    }
    if kind == AssetKind::Skills && resource_dir.join("agents").join("openai.yaml").exists() {
        return Err(anyhow!(
            "skill `{}` must not author `agents/openai.yaml` directly; use Codex frontmatter metadata instead",
            logical_name
        ));
    }

    let markdown = fs::read_to_string(&primary_path)?;
    let resource = parse_prompt_resource(kind, &managed_name, &markdown)?;

    match (adapter, kind) {
        (AdapterKind::Claude, AssetKind::Skills) | (AdapterKind::Claude, AssetKind::Agents) => {
            let rendered = render_claude_markdown(kind, &resource)?;
            push_rendered_file(
                project_root,
                adapter,
                kind,
                managed_name.clone(),
                PathBuf::from(&managed_name).join(primary_name),
                rendered.into_bytes(),
                origin_layer,
                origin_detail.clone(),
                plan,
                seen,
                ExposureMode::Direct,
            )?;
            copy_prompt_directory_companions(
                project_root,
                adapter,
                kind,
                resource_dir,
                &managed_name,
                &primary_path,
                origin_layer,
                origin_detail,
                plan,
                seen,
            )
        }
        (AdapterKind::Codex, AssetKind::Skills) => {
            let rendered = render_codex_prompt_markdown(&resource);
            push_rendered_file(
                project_root,
                adapter,
                kind,
                managed_name.clone(),
                PathBuf::from(&managed_name).join(primary_name),
                rendered.into_bytes(),
                origin_layer,
                origin_detail.clone(),
                plan,
                seen,
                ExposureMode::Direct,
            )?;
            copy_prompt_directory_companions(
                project_root,
                adapter,
                kind,
                resource_dir,
                &managed_name,
                &primary_path,
                origin_layer,
                origin_detail.clone(),
                plan,
                seen,
            )?;
            if let Some(sidecar) = render_codex_skill_sidecar(&resource)? {
                push_rendered_file(
                    project_root,
                    adapter,
                    kind,
                    managed_name,
                    PathBuf::from(resource.logical_name.as_str())
                        .join("agents")
                        .join("openai.yaml"),
                    sidecar.into_bytes(),
                    origin_layer,
                    origin_detail,
                    plan,
                    seen,
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
            if let Some(index) = seen.get(&generated_relative_path).copied() {
                plan[index] = PlannedFile {
                    adapter: AdapterKind::Codex,
                    kind: AssetKind::Agents,
                    exposure_mode: ExposureMode::GeneratedComposite,
                    relative_name: managed_name,
                    generated_relative_path,
                    exposed_relative_path,
                    contents: rendered.into_bytes(),
                    origin_layer,
                    origin_detail,
                };
            } else {
                let index = plan.len();
                seen.insert(generated_relative_path.clone(), index);
                plan.push(PlannedFile {
                    adapter: AdapterKind::Codex,
                    kind: AssetKind::Agents,
                    exposure_mode: ExposureMode::GeneratedComposite,
                    relative_name: managed_name,
                    generated_relative_path,
                    exposed_relative_path,
                    contents: rendered.into_bytes(),
                    origin_layer,
                    origin_detail,
                });
            }
            Ok(())
        }
        _ => Err(anyhow!(
            "unexpected prompt directory mapping for `{}` `{}`",
            adapter.as_str(),
            kind.as_str()
        )),
    }
}

fn copy_prompt_directory_companions(
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    resource_dir: &Path,
    logical_name: &str,
    primary_path: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    seen: &mut BTreeMap<PathBuf, usize>,
) -> Result<()> {
    let files = collect_file_paths(resource_dir)?;
    for file in files {
        if file == primary_path || is_asset_metadata_file(&file) {
            continue;
        }
        let rel = file.strip_prefix(resource_dir)?;
        if kind == AssetKind::Skills && rel == Path::new("agents").join("openai.yaml").as_path() {
            continue;
        }
        push_rendered_file(
            project_root,
            adapter,
            kind,
            logical_name.to_string(),
            PathBuf::from(logical_name).join(rel),
            fs::read(&file)?,
            origin_layer,
            origin_detail.clone(),
            plan,
            seen,
            ExposureMode::Direct,
        )?;
    }
    Ok(())
}

fn push_rendered_file(
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    relative_name: String,
    relative_path_within_kind: PathBuf,
    contents: Vec<u8>,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    seen: &mut BTreeMap<PathBuf, usize>,
    exposure_mode: ExposureMode,
) -> Result<()> {
    let generated_relative_path = PathBuf::from(".ply")
        .join("generated")
        .join(adapter.as_str())
        .join(kind.as_str())
        .join(&relative_path_within_kind);
    let exposed_root = adapter
        .direct_asset_root(project_root, kind)
        .ok_or_else(|| anyhow!("no direct root for `{}` `{}`", adapter.as_str(), kind.as_str()))?;
    let exposed_relative_path = exposed_root
        .strip_prefix(project_root)?
        .join(&relative_path_within_kind);

    if let Some(index) = seen.get(&generated_relative_path).copied() {
        plan[index] = PlannedFile {
            adapter,
            kind,
            exposure_mode,
            relative_name,
            generated_relative_path,
            exposed_relative_path,
            contents,
            origin_layer,
            origin_detail,
        };
    } else {
        let index = plan.len();
        seen.insert(generated_relative_path.clone(), index);
        plan.push(PlannedFile {
            adapter,
            kind,
            exposure_mode,
            relative_name,
            generated_relative_path,
            exposed_relative_path,
            contents,
            origin_layer,
            origin_detail,
        });
    }
    Ok(())
}

fn render_codex_prompt_markdown(resource: &crate::prompt_resources::ParsedPromptResource) -> String {
    match render_codex_prompt_preamble(resource) {
        Some(preamble) if !resource.body.is_empty() => format!("{preamble}{}\n", resource.body),
        Some(preamble) => preamble,
        None if resource.body.is_empty() => String::new(),
        None => format!("{}\n", resource.body),
    }
}

fn collect_planned_files(
    project_root: &Path,
    adapter: AdapterKind,
    kind: AssetKind,
    source_dir: &Path,
    origin_layer: LayerKind,
    origin_detail: String,
    plan: &mut Vec<PlannedFile>,
    seen: &mut BTreeMap<PathBuf, usize>,
) -> Result<()> {
    if adapter == AdapterKind::Codex && kind == AssetKind::Agents {
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
        if !resource_targets_adapter(metadata.as_ref(), adapter)? {
            continue;
        }
        let managed_name = ensure_managed_name(kind, &top_level_name);
        let managed_rel = managed_relative_path(kind, rel)?;

        let generated_relative_path = PathBuf::from(".ply")
            .join("generated")
            .join(adapter.as_str())
            .join(kind.as_str())
            .join(&managed_rel);
        let exposed_root = adapter
            .direct_asset_root(project_root, kind)
            .ok_or_else(|| anyhow!("no direct root for `{}` `{}`", adapter.as_str(), kind.as_str()))?;
        let exposed_relative_path = exposed_root.strip_prefix(project_root)?.join(&managed_rel);

        if let Some(index) = seen.get(&generated_relative_path).copied() {
            plan[index] = PlannedFile {
                adapter,
                kind,
                exposure_mode: ExposureMode::Direct,
                relative_name: managed_name,
                generated_relative_path,
                exposed_relative_path,
                contents: fs::read(&file)?,
                origin_layer,
                origin_detail: origin_detail.clone(),
            };
        } else {
            let index = plan.len();
            seen.insert(generated_relative_path.clone(), index);
            plan.push(PlannedFile {
                adapter,
                kind,
                exposure_mode: ExposureMode::Direct,
                relative_name: managed_name,
                generated_relative_path,
                exposed_relative_path,
                contents: fs::read(&file)?,
                origin_layer,
                origin_detail: origin_detail.clone(),
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
    if let Some(start) = existing.find(PLY_MANAGED_START) {
        if let Some(end) = existing[start..].find(PLY_MANAGED_END) {
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

fn load_resource_metadata(source_dir: &Path, top_level_name: &str) -> Result<Option<AssetMetadata>> {
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

fn resource_targets_adapter(metadata: Option<&AssetMetadata>, adapter: AdapterKind) -> Result<bool> {
    let Some(metadata) = metadata else {
        return Ok(true);
    };
    if metadata.targets.is_empty() {
        return Ok(true);
    }
    for target in &metadata.targets {
        AdapterKind::parse(target)?;
    }
    Ok(metadata.targets.iter().any(|target| target == adapter.as_str()))
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
            manifest.sources.push(materialize_local_source(local_source)?);
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
    let kind = local
        .kind
        .ok_or_else(|| anyhow!("local source `{}` must define `kind` when adding a new source", local.id))?;
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
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        assert!(report.target_root.join("ply.toml").exists());
        assert!(report.target_root.join("ply.local.toml").exists());
        assert!(
            report
                .target_root
                .join(".ply")
                .join("state.json")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn init_dry_run_does_not_create_files() -> Result<()> {
        let temp = make_project()?;
        let global = temp.path().join("global-root");
        unsafe {
            std::env::set_var("HOME", temp.path());
        }
        let report = init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: false,
                    ignore_config: false,
                },
                dry_run: true,
                target: CommandTarget::Global,
            },
        )?;
        assert!(report.dry_run);
        assert!(!global.join("ply.toml").exists());
        Ok(())
    }

    #[test]
    fn apply_copies_assets_from_path_source() -> Result<()> {
        let temp = make_project()?;
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
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
            "schema_version = 1\nadapters = [\"codex\", \"claude\"]\n\n[install]\nmode = \"copy\"\nuse_global = false\n\n[[sources]]\nid = \"fixture\"\nkind = \"path\"\npath = \"{}\"\n",
            package_root.display()
        );
        write(&temp.path().join("ply.toml"), &manifest)?;
        write(&temp.path().join("ply.local.toml"), "schema_version = 1\n")?;
        write(
            &temp.path().join(".ply").join("state.json"),
            "{\n  \"schema_version\": 1,\n  \"install_mode\": \"copy\",\n  \"ignore_config\": false,\n  \"owned_paths\": []\n}\n",
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
            !temp.path()
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
            !temp.path()
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
            !temp.path()
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
            !temp.path()
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
    fn apply_dry_run_reports_drift() -> Result<()> {
        let temp = make_project()?;
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
            &package_root
                .join("output-styles")
                .join("ply-review.md"),
            "Be concise and bug-focused.\n",
        )?;
        write(
            &package_root
                .join("rules")
                .join("ply-safe.md"),
            "Never mutate tracked files without consent.\n",
        )?;
        write(
            &package_root
                .join("hooks")
                .join("ply-lint.sh"),
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        let package_root = example_package_root(temp.path());
        write(
            &temp.path().join("CLAUDE.local.md"),
            "Personal note.\n",
        )?;
        write(
            &package_root.join("local-instructions.md"),
            "Work through diffs carefully.\n",
        )?;
        write(
            &package_root
                .join("output-styles")
                .join("ply-review.md"),
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        let package_root = example_package_root(temp.path());

        write(
            &package_root.join("commands").join("docs.md"),
            r#"---
name: docs-helper
description: Help with project documentation
argument-hint: [topic]
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
            &package_root
                .join("output-styles")
                .join("concise.md"),
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

        let claude_command =
            fs::read_to_string(temp.path().join(".claude").join("commands").join("ply-docs.md"))?;
        assert!(claude_command.contains("allowed-tools:"));
        assert!(claude_command.contains("argument-hint:"));
        assert!(claude_command.contains("topic"));

        let codex_command =
            fs::read_to_string(temp.path().join(".agents").join("commands").join("ply-docs.md"))?;
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
        let codex_agent =
            fs::read_to_string(temp.path().join(".codex").join("agents").join("ply-reviewer.toml"))?;
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
        assert!(err.to_string().contains("must not author `agents/openai.yaml` directly"));
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
        assert!(temp.path().join("review-tools").join("ply-package.toml").exists());
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
    fn diff_groups_exposed_changes() -> Result<()> {
        let temp = make_project()?;
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
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
        init_project(
            temp.path(),
            InitRequest {
                options: InitOptions {
                    scaffold_local_packages: true,
                    ignore_config: false,
                },
                dry_run: false,
                target: CommandTarget::Project,
            },
        )?;
        crate::git::remove_local_excludes(temp.path())?;

        let report = doctor(temp.path(), CommandTarget::Project)?;
        assert!(report.contains("Warnings:"));
        assert!(report.contains("missing Ply block in .git/info/exclude"));
        Ok(())
    }
}
