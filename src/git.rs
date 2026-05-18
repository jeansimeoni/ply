use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{InitOptions, SourceConfig};

const PLY_EXCLUDE_START: &str = "# ply:start";
const PLY_EXCLUDE_END: &str = "# ply:end";

pub fn ensure_git_repo(project_root: &Path) -> Result<()> {
    run_git(project_root, ["rev-parse", "--show-toplevel"])?;
    Ok(())
}

pub fn is_git_repo(project_root: &Path) -> bool {
    run_git(project_root, ["rev-parse", "--show-toplevel"]).is_ok()
}

pub fn ensure_local_excludes(project_root: &Path, options: InitOptions) -> Result<()> {
    if !is_git_repo(project_root) {
        return Ok(());
    }
    let exclude_path = project_root.join(".git").join("info").join("exclude");
    let mut block = String::from(
        r#"# ply:start
.ply/generated/
.ply/cache/
.ply/state.json
.ply/local.yml
imp-plan/
AGENTS.override.md
CLAUDE.local.md
.claude/commands/ply-*.md
.claude/hooks/ply-*
.claude/output-styles/ply-*
.claude/rules/ply-*
.claude/skills/ply-*/
.agents/commands/ply-*.md
.agents/skills/ply-*/
.codex/hooks.json
.codex/hooks/ply-*
.codex/rules/ply-*
"#,
    );
    if options.ignore_config {
        block.push_str(".ply/\n");
        block.push_str("ply.toml\n");
        block.push_str("ply.lock\n");
        block.push_str("ply-packages/\n");
    }
    block.push_str("# ply:end\n");
    let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
    if existing.contains(PLY_EXCLUDE_START) {
        return Ok(());
    }
    let mut content = existing;
    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&block);
    fs::write(&exclude_path, content)
        .with_context(|| format!("failed to update {}", exclude_path.display()))
}

pub fn has_ply_excludes(project_root: &Path) -> bool {
    if !is_git_repo(project_root) {
        return false;
    }
    let exclude_path = project_root.join(".git").join("info").join("exclude");
    fs::read_to_string(exclude_path)
        .map(|content| content.contains(PLY_EXCLUDE_START))
        .unwrap_or(false)
}

pub fn remove_local_excludes(project_root: &Path) -> Result<bool> {
    if !is_git_repo(project_root) {
        return Ok(false);
    }
    let exclude_path = project_root.join(".git").join("info").join("exclude");
    let existing = match fs::read_to_string(&exclude_path) {
        Ok(content) => content,
        Err(_) => return Ok(false),
    };
    let Some(start) = existing.find(PLY_EXCLUDE_START) else {
        return Ok(false);
    };
    let Some(end_marker) = existing[start..].find(PLY_EXCLUDE_END) else {
        return Ok(false);
    };
    let mut end = start + end_marker + PLY_EXCLUDE_END.len();
    if existing[end..].starts_with('\n') {
        end += 1;
    }

    let mut updated = String::new();
    updated.push_str(&existing[..start]);
    updated.push_str(&existing[end..]);
    let updated = updated.trim_matches('\n');
    let final_content = if updated.is_empty() {
        String::new()
    } else {
        format!("{updated}\n")
    };

    fs::write(&exclude_path, final_content)
        .with_context(|| format!("failed to update {}", exclude_path.display()))?;
    Ok(true)
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

pub fn diff_contents(current: &[u8], desired: &[u8], label: &str) -> Result<String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!("ply-diff-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&temp_root)?;
    let current_path = temp_root.join("current");
    let desired_path = temp_root.join("desired");
    fs::write(&current_path, current)?;
    fs::write(&desired_path, desired)?;

    let output = Command::new("git")
        .args(["diff", "--no-index", "--no-prefix", "--"])
        .arg(&current_path)
        .arg(&desired_path)
        .output()
        .context("failed to run `git diff --no-index`")?;

    let _ = fs::remove_dir_all(&temp_root);

    if !output.status.success() && output.status.code() != Some(1) {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let diff = String::from_utf8_lossy(&output.stdout)
        .replace(
            current_path.to_string_lossy().as_ref(),
            &format!("a/{label}"),
        )
        .replace(
            desired_path.to_string_lossy().as_ref(),
            &format!("b/{label}"),
        );
    Ok(diff.trim().to_string())
}
