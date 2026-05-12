use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::SourceConfig;

pub fn ensure_git_repo(project_root: &Path) -> Result<()> {
    run_git(project_root, ["rev-parse", "--show-toplevel"])?;
    Ok(())
}

pub fn ensure_local_excludes(project_root: &Path) -> Result<()> {
    let exclude_path = project_root.join(".git").join("info").join("exclude");
    let block = r#"# ply:start
.ply/generated/
.ply/cache/
.ply/state.json
.ply/local.yml
imp-plan/
.claude/commands/ply-*.md
.claude/skills/ply-*/
.agents/commands/ply-*.md
.agents/skills/ply-*/
# ply:end
"#;
    let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
    if existing.contains("# ply:start") {
        return Ok(());
    }
    let mut content = existing;
    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(block);
    fs::write(&exclude_path, content)
        .with_context(|| format!("failed to update {}", exclude_path.display()))
}

pub fn is_tracked(project_root: &Path, path: &Path) -> Result<bool> {
    let relative = relative_path(project_root, path)?;
    let output = Command::new("git")
        .args(["ls-files", "--error-unmatch"])
        .arg(relative)
        .current_dir(project_root)
        .output()
        .context("failed to run `git ls-files`")?;
    Ok(output.status.success())
}

pub fn is_ignored(project_root: &Path, path: &Path) -> Result<bool> {
    let relative = relative_path(project_root, path)?;
    let status = Command::new("git")
        .args(["check-ignore", "-q"])
        .arg(relative)
        .current_dir(project_root)
        .status()
        .context("failed to run `git check-ignore`")?;
    Ok(status.success())
}

pub fn clone_or_update_source(
    project_root: &Path,
    source: &SourceConfig,
) -> Result<(PathBuf, String)> {
    let cache_root = project_root.join(".ply").join("cache").join("sources");
    fs::create_dir_all(&cache_root)?;
    let repo_path = cache_root.join(&source.id);
    let url = source
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("git source `{}` missing url", source.id))?;

    if !repo_path.exists() {
        let status = Command::new("git")
            .args(["clone", url])
            .arg(&repo_path)
            .current_dir(project_root)
            .status()
            .context("failed to run `git clone`")?;
        if !status.success() {
            return Err(anyhow!("git clone failed for source `{}`", source.id));
        }
    } else {
        let _ = Command::new("git")
            .args(["fetch", "--all", "--tags", "--prune"])
            .current_dir(&repo_path)
            .status();
    }

    let rev = source.rev.as_deref().unwrap_or("HEAD");
    let resolved = run_git(&repo_path, ["rev-parse", rev])?;
    let checkout_status = Command::new("git")
        .args(["checkout", "--force", resolved.trim()])
        .current_dir(&repo_path)
        .status()
        .context("failed to run `git checkout`")?;
    if !checkout_status.success() {
        return Err(anyhow!(
            "failed to checkout revision `{}` for source `{}`",
            resolved.trim(),
            source.id
        ));
    }

    Ok((repo_path, resolved.trim().to_string()))
}

pub fn run_git<I, S>(project_root: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .context("failed to run git command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn relative_path<'a>(project_root: &'a Path, path: &'a Path) -> Result<&'a Path> {
    path.strip_prefix(project_root)
        .with_context(|| format!("{} is not under {}", path.display(), project_root.display()))
}
