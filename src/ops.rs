use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::{AdapterKind, AssetKind};
use crate::config::{
    self, InitOptions, LocalOverlayConfig, LockedSource, Lockfile, Manifest, OverlayEntry,
    OwnedPath, PackageManifest, PackageSelection, SourceConfig, State, load_local_overlays,
    load_manifest, load_manifest_if_present, load_package_manifest, load_state,
};
use crate::git;
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
    overlays: LocalOverlayConfig,
}

#[derive(Debug, Clone)]
struct MergedSource {
    config: SourceConfig,
    root: PathBuf,
    layer: LayerKind,
}

#[derive(Debug, Clone)]
struct MergedPackage {
    selection: PackageSelection,
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
    packages: Vec<MergedPackage>,
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
        config::write_default_local_overlay(&target_root)?;
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
                relative_name: file.relative_name.clone(),
                generated_path: file.generated_relative_path.to_string_lossy().to_string(),
                exposed_path: file.exposed_relative_path.to_string_lossy().to_string(),
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

    let mut lines = Vec::new();
    for file in &planned_files {
        let generated = generated_abs_path(project_root, file);
        let exposed = exposed_abs_path(project_root, file);
        if !generated.exists() {
            lines.push(ui::status_line(
                Tone::Info,
                &format!("generate {}", file.generated_relative_path.display()),
            ));
        }
        if !exposed.exists() {
            lines.push(ui::status_line(
                Tone::Info,
                &format!("expose {}", file.exposed_relative_path.display()),
            ));
            continue;
        }
        let current = fs::read(&exposed)?;
        if current != file.contents {
            lines.push(ui::status_line(
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
                lines.push(diff);
            }
        }
    }

    for stale in owned_previous.difference(&desired_exposed) {
        lines.push(ui::status_line(
            Tone::Warning,
            &format!("remove {}", stale.strip_prefix(project_root)?.display()),
        ));
    }
    for generated_path in collect_file_paths(&project_root.join(".ply").join("generated"))? {
        if !desired_generated.contains(&generated_path) {
            lines.push(ui::status_line(
                Tone::Warning,
                &format!(
                    "remove {}",
                    generated_path.strip_prefix(project_root)?.display()
                ),
            ));
        }
    }

    if lines.is_empty() {
        return Ok("no differences".to_string());
    }
    Ok(lines.join("\n\n"))
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
    let mut lines = vec![ui::status_line(Tone::Success, "manifest parsed")];
    lines.push(ui::status_line(
        Tone::Success,
        &format!("{} source(s) resolved", sources.len()),
    ));
    lines.push(ui::status_line(
        Tone::Success,
        &format!("{} package(s) resolved", packages.len()),
    ));
    lines.push(ui::status_line(
        Tone::Success,
        &format!("{} managed file(s) planned", planned_files.len()),
    ));
    for file in &planned_files {
        let exposed = exposed_abs_path(&root, file);
        if git::is_tracked(&root, &exposed)? {
            lines.push(ui::status_line(
                Tone::Warning,
                &format!("tracked target {}", file.exposed_relative_path.display()),
            ));
        }
        if !git::is_ignored(&root, &exposed)? {
            lines.push(ui::status_line(
                Tone::Warning,
                &format!("unignored target {}", file.exposed_relative_path.display()),
            ));
        }
    }
    Ok(lines.join("\n"))
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
            "{} ({}) from {} [{}]",
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
    let project_overlays = load_local_overlays(project_root)?;
    let mut layers = Vec::new();

    if project_manifest.install.use_global {
        let global_root = config::global_root()?;
        if let Some(global_manifest) = load_manifest_if_present(&global_root)? {
            let overlays = load_local_overlays(&global_root)?;
            layers.push(LayerConfig {
                kind: LayerKind::Global,
                root: global_root,
                manifest: global_manifest,
                overlays,
            });
        }
    }

    layers.push(LayerConfig {
        kind: LayerKind::Project,
        root: project_root.to_path_buf(),
        manifest: project_manifest,
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
    let overlays = load_local_overlays(root)?;
    compose_layers(vec![LayerConfig {
        kind: layer,
        root: root.to_path_buf(),
        manifest,
        overlays,
    }])
}

fn compose_layers(layers: Vec<LayerConfig>) -> Result<ComposedConfig> {
    let mut adapters = Vec::new();
    let mut seen_adapters = BTreeSet::new();
    let mut source_index = BTreeMap::new();
    let mut sources = Vec::new();
    let mut packages = Vec::new();
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
                    root: layer.root.clone(),
                    layer: layer.kind,
                };
            } else {
                sources.push(MergedSource {
                    config: source.clone(),
                    root: layer.root.clone(),
                    layer: layer.kind,
                });
            }
        }

        for package in &layer.manifest.packages {
            packages.push(MergedPackage {
                selection: package.clone(),
            });
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
        packages,
        overlays,
    })
}

fn resolve_composed(
    config: &ComposedConfig,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let mut sources = Vec::new();
    let mut source_by_id = BTreeMap::new();
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
                let (root, revision) = git::clone_or_update_source(&source.root, &source.config)?;
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
        source_by_id.insert(resolved.id.clone(), resolved.clone());
        sources.push(resolved);
    }

    let mut packages = Vec::new();
    for selection in &config.packages {
        let source = source_by_id
            .get(&selection.selection.source)
            .ok_or_else(|| anyhow!("unknown source `{}`", selection.selection.source))?;
        let package_root = source.root.join(&selection.selection.path);
        if !package_root.exists() {
            return Err(anyhow!(
                "package path `{}` does not exist in source `{}`",
                selection.selection.path,
                selection.selection.source
            ));
        }
        let manifest = load_package_manifest(&package_root)?;
        packages.push(ResolvedPackage {
            source_id: selection.selection.source.clone(),
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

    for adapter_name in adapter_names {
        let adapter = AdapterKind::parse(adapter_name)?;
        for package in packages {
            let base = package.root.join(adapter.as_str());
            for kind in [AssetKind::Commands, AssetKind::Skills] {
                let source_dir = base.join(kind.as_str());
                if source_dir.exists() {
                    collect_planned_files(
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
                        &mut seen,
                    )?;
                }
            }
        }
    }

    for overlay in overlays {
        let adapter = AdapterKind::parse(&overlay.entry.adapter)?;
        let kind = AssetKind::parse(&overlay.entry.kind)?;
        let source_dir = overlay.root.join(&overlay.entry.path);
        if source_dir.exists() {
            collect_planned_files(
                project_root,
                adapter,
                kind,
                &source_dir,
                overlay.layer,
                format!("overlay {}", overlay.entry.path),
                &mut plan,
                &mut seen,
            )?;
        }
    }

    plan.sort_by(|a, b| a.generated_relative_path.cmp(&b.generated_relative_path));
    Ok(plan)
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
    for file in collect_file_paths(source_dir)? {
        let rel = file.strip_prefix(source_dir)?;
        let top_level_name = rel
            .components()
            .next()
            .ok_or_else(|| anyhow!("empty relative path under {}", source_dir.display()))?
            .as_os_str()
            .to_string_lossy()
            .to_string();
        if !top_level_name.starts_with("ply-") {
            return Err(anyhow!(
                "managed asset `{}` must use the `ply-` prefix",
                rel.display()
            ));
        }

        let generated_relative_path = PathBuf::from(".ply")
            .join("generated")
            .join(adapter.as_str())
            .join(kind.as_str())
            .join(rel);
        let exposed_root = adapter.asset_root(project_root, kind);
        let exposed_relative_path = exposed_root.strip_prefix(project_root)?.join(rel);

        if let Some(index) = seen.get(&generated_relative_path).copied() {
            plan[index] = PlannedFile {
                adapter,
                kind,
                relative_name: top_level_name,
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
                relative_name: top_level_name,
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
        if path.exists() && !previous_owned.contains(&path) {
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
        (AdapterKind::Claude, AssetKind::Commands),
        (AdapterKind::Claude, AssetKind::Skills),
    ] {
        let root = adapter.asset_root(project_root, kind);
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

    fn make_project() -> Result<TempDir> {
        let temp = TempDir::new()?;
        exec_in(temp.path(), &["init"])?;
        exec_in(temp.path(), &["config", "user.email", "test@example.com"])?;
        exec_in(temp.path(), &["config", "user.name", "Test User"])?;
        Ok(temp)
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
        assert!(report.target_root.join(".ply").join("local.yml").exists());
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
}
