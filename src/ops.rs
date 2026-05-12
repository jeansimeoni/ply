use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::{AdapterKind, AssetKind};
use crate::config::{
    self, LocalOverlayConfig, LockedSource, Lockfile, OwnedPath, PackageManifest, State,
    load_local_overlays, load_manifest, load_package_manifest, load_state,
};
use crate::git;

#[derive(Debug, Clone)]
struct ResolvedSource {
    id: String,
    kind: String,
    root: PathBuf,
    resolved: String,
}

#[derive(Debug, Clone)]
struct ResolvedPackage {
    source_id: String,
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
}

pub fn init_project(project_root: &Path) -> Result<()> {
    git::ensure_git_repo(project_root)?;
    git::ensure_local_excludes(project_root)?;
    fs::create_dir_all(project_root.join(".ply").join("generated"))?;
    fs::create_dir_all(
        project_root
            .join(".ply")
            .join("overlays")
            .join("codex")
            .join("skills"),
    )?;
    fs::create_dir_all(
        project_root
            .join(".ply")
            .join("overlays")
            .join("claude")
            .join("skills"),
    )?;
    config::write_default_manifest(project_root)?;
    config::write_default_local_overlay(project_root)?;
    config::write_state(
        project_root,
        &State {
            schema_version: 1,
            install_mode: "copy".to_string(),
            owned_paths: Vec::new(),
        },
    )?;
    config::write_default_package_fixture(project_root)?;
    Ok(())
}

pub fn apply(project_root: &Path) -> Result<String> {
    git::ensure_git_repo(project_root)?;
    git::ensure_local_excludes(project_root)?;
    let manifest = load_manifest(project_root)?;
    let overlays = load_local_overlays(project_root)?;
    let previous_state = load_state(project_root)?;
    let (sources, packages) = resolve(project_root, &manifest)?;
    let planned_files = build_plan(project_root, &manifest.adapters, &packages, &overlays)?;
    verify_exposed_targets(project_root, &planned_files, &previous_state)?;
    write_generated_tree(project_root, &planned_files)?;
    remove_stale_paths(project_root, &previous_state, &planned_files)?;
    write_exposed_tree(project_root, &planned_files)?;
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
        install_mode: manifest.install.mode.clone(),
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
    Ok(format!(
        "applied {} managed file(s) across {} source(s)",
        planned_files.len(),
        sources.len()
    ))
}

pub fn diff(project_root: &Path) -> Result<String> {
    let manifest = load_manifest(project_root)?;
    let overlays = load_local_overlays(project_root)?;
    let previous_state = load_state(project_root)?;
    let (_, packages) = resolve(project_root, &manifest)?;
    let planned_files = build_plan(project_root, &manifest.adapters, &packages, &overlays)?;

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
            lines.push(format!(
                "generate {}",
                file.generated_relative_path.display()
            ));
        } else if fs::read(&generated)? != file.contents {
            lines.push(format!("update {}", file.generated_relative_path.display()));
        }
        if !exposed.exists() {
            lines.push(format!("expose {}", file.exposed_relative_path.display()));
        } else if fs::read(&exposed)? != file.contents {
            lines.push(format!("refresh {}", file.exposed_relative_path.display()));
        }
    }

    for stale in owned_previous.difference(&desired_exposed) {
        lines.push(format!(
            "remove {}",
            stale.strip_prefix(project_root)?.display()
        ));
    }
    for generated_path in collect_file_paths(&project_root.join(".ply").join("generated"))? {
        if !desired_generated.contains(&generated_path) {
            lines.push(format!(
                "remove {}",
                generated_path.strip_prefix(project_root)?.display()
            ));
        }
    }

    if lines.is_empty() {
        return Ok("no differences".to_string());
    }
    Ok(lines.join("\n"))
}

pub fn doctor(project_root: &Path) -> Result<String> {
    let manifest = load_manifest(project_root)?;
    let overlays = load_local_overlays(project_root)?;
    let (sources, packages) = resolve(project_root, &manifest)?;
    let planned_files = build_plan(project_root, &manifest.adapters, &packages, &overlays)?;
    let mut lines = vec!["[OK] manifest parsed".to_string()];
    lines.push(format!("[OK] {} source(s) resolved", sources.len()));
    lines.push(format!("[OK] {} package(s) resolved", packages.len()));
    lines.push(format!(
        "[OK] {} managed file(s) planned",
        planned_files.len()
    ));
    for file in &planned_files {
        let exposed = exposed_abs_path(project_root, file);
        if git::is_tracked(project_root, &exposed)? {
            lines.push(format!(
                "[WARN] tracked target {}",
                file.exposed_relative_path.display()
            ));
        }
        if !git::is_ignored(project_root, &exposed)? {
            lines.push(format!(
                "[WARN] unignored target {}",
                file.exposed_relative_path.display()
            ));
        }
    }
    Ok(lines.join("\n"))
}

pub fn list_packages(project_root: &Path) -> Result<String> {
    let manifest = load_manifest(project_root)?;
    let (_, packages) = resolve(project_root, &manifest)?;
    let mut lines = Vec::new();
    for package in packages {
        lines.push(format!(
            "{} ({}) from {}",
            package.manifest.name,
            package
                .manifest
                .version
                .unwrap_or_else(|| "unversioned".to_string()),
            package.source_id
        ));
    }
    Ok(lines.join("\n"))
}

pub fn list_sources(project_root: &Path) -> Result<String> {
    let manifest = load_manifest(project_root)?;
    let (sources, _) = resolve(project_root, &manifest)?;
    let mut lines = Vec::new();
    for source in sources {
        lines.push(format!(
            "{} [{}] {}",
            source.id, source.kind, source.resolved
        ));
    }
    Ok(lines.join("\n"))
}

fn resolve(
    project_root: &Path,
    manifest: &config::Manifest,
) -> Result<(Vec<ResolvedSource>, Vec<ResolvedPackage>)> {
    let mut sources = Vec::new();
    let mut source_by_id = BTreeMap::new();
    for source in &manifest.sources {
        let resolved = match source.kind.as_str() {
            "path" => {
                let path = project_root.join(
                    source
                        .path
                        .as_deref()
                        .ok_or_else(|| anyhow!("path source `{}` missing path", source.id))?,
                );
                let root = path.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve path source `{}` at {}",
                        source.id,
                        path.display()
                    )
                })?;
                ResolvedSource {
                    id: source.id.clone(),
                    kind: source.kind.clone(),
                    resolved: root.display().to_string(),
                    root,
                }
            }
            "git" => {
                let (root, revision) = git::clone_or_update_source(project_root, source)?;
                ResolvedSource {
                    id: source.id.clone(),
                    kind: source.kind.clone(),
                    resolved: revision,
                    root,
                }
            }
            other => return Err(anyhow!("unsupported source kind `{other}`")),
        };
        source_by_id.insert(resolved.id.clone(), resolved.root.clone());
        sources.push(resolved);
    }

    let mut packages = Vec::new();
    let mut seen_names = BTreeSet::new();
    for selection in &manifest.packages {
        let source_root = source_by_id
            .get(&selection.source)
            .ok_or_else(|| anyhow!("unknown source `{}`", selection.source))?;
        let package_root = source_root.join(&selection.path);
        if !package_root.exists() {
            return Err(anyhow!(
                "package path `{}` does not exist in source `{}`",
                selection.path,
                selection.source
            ));
        }
        let manifest = load_package_manifest(&package_root)?;
        if !seen_names.insert(manifest.name.clone()) {
            return Err(anyhow!(
                "duplicate package name `{}` in resolved set",
                manifest.name
            ));
        }
        packages.push(ResolvedPackage {
            source_id: selection.source.clone(),
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
    overlays: &LocalOverlayConfig,
) -> Result<Vec<PlannedFile>> {
    let mut plan = Vec::new();
    let mut seen = BTreeSet::new();

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
                        &mut plan,
                        &mut seen,
                    )?;
                }
            }
        }
    }

    for overlay in &overlays.overlays {
        let adapter = AdapterKind::parse(&overlay.adapter)?;
        let kind = AssetKind::parse(&overlay.kind)?;
        let source_dir = project_root.join(&overlay.path);
        if source_dir.exists() {
            collect_planned_files(
                project_root,
                adapter,
                kind,
                &source_dir,
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
    plan: &mut Vec<PlannedFile>,
    seen: &mut BTreeSet<PathBuf>,
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
        if !seen.insert(generated_relative_path.clone()) {
            return Err(anyhow!(
                "collision while generating `{}`",
                generated_relative_path.display()
            ));
        }
        let exposed_root = adapter.asset_root(project_root, kind);
        let exposed_relative_path = exposed_root.strip_prefix(project_root)?.join(rel);
        plan.push(PlannedFile {
            adapter,
            kind,
            relative_name: top_level_name,
            generated_relative_path,
            exposed_relative_path,
            contents: fs::read(&file)?,
        });
    }
    Ok(())
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

fn write_exposed_tree(project_root: &Path, planned_files: &[PlannedFile]) -> Result<()> {
    for file in planned_files {
        let path = exposed_abs_path(project_root, file);
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
        init_project(temp.path())?;
        assert!(temp.path().join("ply.toml").exists());
        assert!(temp.path().join(".ply").join("local.yml").exists());
        assert!(temp.path().join(".ply").join("state.json").exists());
        Ok(())
    }

    #[test]
    fn apply_copies_assets_from_path_source() -> Result<()> {
        let temp = make_project()?;
        init_project(temp.path())?;
        apply(temp.path())?;
        let skill = temp
            .path()
            .join(".agents")
            .join("skills")
            .join("ply-review-diff")
            .join("SKILL.md");
        assert!(skill.exists());
        Ok(())
    }

    #[test]
    fn apply_refuses_tracked_conflict() -> Result<()> {
        let temp = make_project()?;
        init_project(temp.path())?;
        let tracked = temp
            .path()
            .join(".agents")
            .join("skills")
            .join("ply-review-diff")
            .join("SKILL.md");
        write(&tracked, "tracked\n")?;
        exec_in(temp.path(), &["add", "ply.toml"])?;
        exec_in(
            temp.path(),
            &["add", "-f", ".agents/skills/ply-review-diff/SKILL.md"],
        )?;
        exec_in(temp.path(), &["commit", "-m", "track conflict"])?;
        let err = apply(temp.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("refusing to overwrite tracked path")
        );
        Ok(())
    }

    #[test]
    fn apply_supports_local_git_source() -> Result<()> {
        let source_repo = make_project()?;
        exec_in(source_repo.path(), &["init"])?;
        exec_in(
            source_repo.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        exec_in(source_repo.path(), &["config", "user.name", "Test User"])?;
        let package_root = source_repo.path().join("pkg").join("review");
        write(
            &package_root.join("ply-package.toml"),
            "name = \"ply-git-review\"\n",
        )?;
        write(
            &package_root
                .join("codex")
                .join("skills")
                .join("ply-git-review")
                .join("SKILL.md"),
            "# skill\n",
        )?;
        exec_in(source_repo.path(), &["add", "."])?;
        exec_in(source_repo.path(), &["commit", "-m", "add package"])?;

        let project = make_project()?;
        let manifest = format!(
            "schema_version = 1\nadapters = [\"codex\"]\n\n[install]\nmode = \"copy\"\n\n[[sources]]\nid = \"gitpkg\"\nkind = \"git\"\nurl = \"{}\"\n\n[[packages]]\nsource = \"gitpkg\"\npath = \"pkg/review\"\n",
            source_repo.path().display()
        );
        write(&project.path().join("ply.toml"), &manifest)?;
        write(
            &project.path().join(".ply").join("local.yml"),
            "overlays: []\n",
        )?;

        apply(project.path())?;
        assert!(
            project
                .path()
                .join(".agents")
                .join("skills")
                .join("ply-git-review")
                .join("SKILL.md")
                .exists()
        );
        Ok(())
    }
}
