use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{InitOptions, SourceConfig, SshSourceConfig};

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
ply.local.toml
ply.ssh.toml
imp-plan/
AGENTS.override.md
CLAUDE.local.md
.claude/commands/ply-*.md
.claude/agents/ply-*/
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
    if let Some(start) = existing.find(PLY_EXCLUDE_START) {
        let Some(end_marker) = existing[start..].find(PLY_EXCLUDE_END) else {
            return Err(anyhow!(
                "found `{PLY_EXCLUDE_START}` in {} without matching `{PLY_EXCLUDE_END}`",
                exclude_path.display()
            ));
        };
        let mut end = start + end_marker + PLY_EXCLUDE_END.len();
        if existing[end..].starts_with('\n') {
            end += 1;
        }

        let mut content = String::new();
        content.push_str(&existing[..start]);
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&block);
        let suffix = existing[end..].trim_start_matches('\n');
        if !suffix.is_empty() {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(suffix);
            if !content.ends_with('\n') {
                content.push('\n');
            }
        }
        return fs::write(&exclude_path, content)
            .with_context(|| format!("failed to update {}", exclude_path.display()));
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
    ssh_config: Option<&SshSourceConfig>,
) -> Result<(PathBuf, String)> {
    let cache_root = project_root.join(".ply").join("cache").join("sources");
    fs::create_dir_all(&cache_root)?;
    let repo_path = cache_root.join(&source.id);
    let repo = source
        .repo
        .as_deref()
        .or(source.url.as_deref())
        .ok_or_else(|| anyhow!("git source `{}` missing repo", source.id))?;

    match resolve_repo_spec(project_root, repo)? {
        ResolvedRepoSpec::LocalPath(path) => {
            if let Some(rev) = source.rev.as_deref() {
                if rev != "HEAD" {
                    return Err(anyhow!(
                        "git source `{}` uses a local repo path and does not yet support rev `{}`; omit `rev` or use `HEAD`",
                        source.id,
                        rev
                    ));
                }
            }
            let resolved = run_git(&path, ["rev-parse", "HEAD"])?;
            Ok((path, resolved.trim().to_string()))
        }
        ResolvedRepoSpec::GitHubShorthand(slug) => {
            let remote = if ssh_config.map(|config| config.use_ssh).unwrap_or(false) {
                format!("git@github.com:{slug}.git")
            } else {
                format!("https://github.com/{slug}")
            };
            clone_or_refresh_remote(project_root, source, &repo_path, &remote, ssh_config)
        }
        ResolvedRepoSpec::Remote(remote) => {
            clone_or_refresh_remote(project_root, source, &repo_path, &remote, ssh_config)
        }
    }
}

#[derive(Debug)]
enum ResolvedRepoSpec {
    LocalPath(PathBuf),
    GitHubShorthand(String),
    Remote(String),
}

fn resolve_repo_spec(project_root: &Path, repo: &str) -> Result<ResolvedRepoSpec> {
    if repo.starts_with("git@") || repo.starts_with("ssh://") || repo.contains("://") {
        return Ok(ResolvedRepoSpec::Remote(repo.to_string()));
    }

    let repo_path = PathBuf::from(repo);
    if repo_path.is_absolute() || repo.starts_with("./") || repo.starts_with("../") {
        return Ok(ResolvedRepoSpec::LocalPath(project_root.join(repo).canonicalize().with_context(
            || format!("failed to resolve local repo path {}", project_root.join(repo).display()),
        )?));
    }

    let candidate = project_root.join(repo);
    if candidate.exists() {
        return Ok(ResolvedRepoSpec::LocalPath(candidate.canonicalize().with_context(
            || format!("failed to resolve local repo path {}", candidate.display()),
        )?));
    }

    if repo.matches('/').count() == 1 {
        return Ok(ResolvedRepoSpec::GitHubShorthand(repo.to_string()));
    }

    Err(anyhow!("unsupported git repo spec `{repo}`"))
}

fn clone_or_refresh_remote(
    project_root: &Path,
    source: &SourceConfig,
    repo_path: &Path,
    remote: &str,
    ssh_config: Option<&SshSourceConfig>,
) -> Result<(PathBuf, String)> {
    let ssh_command = ssh_command(ssh_config)?;
    if !repo_path.exists() {
        let mut command = Command::new("git");
        command.args(["clone", remote]).arg(repo_path).current_dir(project_root);
        if let Some(ssh_command) = &ssh_command {
            command.env("GIT_SSH_COMMAND", ssh_command);
        }
        let status = command.status().context("failed to run `git clone`")?;
        if !status.success() {
            return Err(anyhow!("git clone failed for source `{}`", source.id));
        }
    } else {
        let mut command = Command::new("git");
        command
            .args(["fetch", "--all", "--tags", "--prune"])
            .current_dir(repo_path);
        if let Some(ssh_command) = &ssh_command {
            command.env("GIT_SSH_COMMAND", ssh_command);
        }
        let _ = command.status();
    }

    let rev = source.rev.as_deref().unwrap_or("HEAD");
    let resolved = run_git(repo_path, ["rev-parse", rev])?;
    let mut checkout = Command::new("git");
    checkout
        .args(["checkout", "--force", resolved.trim()])
        .current_dir(repo_path);
    if let Some(ssh_command) = &ssh_command {
        checkout.env("GIT_SSH_COMMAND", ssh_command);
    }
    let checkout_status = checkout.status().context("failed to run `git checkout`")?;
    if !checkout_status.success() {
        return Err(anyhow!(
            "failed to checkout revision `{}` for source `{}`",
            resolved.trim(),
            source.id
        ));
    }

    Ok((repo_path.to_path_buf(), resolved.trim().to_string()))
}

fn ssh_command(ssh_config: Option<&SshSourceConfig>) -> Result<Option<String>> {
    let Some(ssh_config) = ssh_config else {
        return Ok(None);
    };
    let key = match (&ssh_config.ssh_key_path, &ssh_config.ssh_key_env) {
        (Some(path), None) => Some(expand_tilde(path)?),
        (None, Some(env_name)) => {
            let value = std::env::var(env_name).with_context(|| {
                format!("environment variable `{env_name}` for ply SSH key is not set")
            })?;
            Some(expand_tilde(&value)?)
        }
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "ssh config cannot define both `ssh_key_path` and `ssh_key_env`"
            ));
        }
    };

    Ok(key.map(|path| format!("ssh -i \"{}\" -o IdentitiesOnly=yes", path.display())))
}

fn expand_tilde(value: &str) -> Result<PathBuf> {
    if let Some(rest) = value.strip_prefix("~/") {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
        return Ok(PathBuf::from(home).join(rest));
    }
    Ok(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ensure_local_excludes_refreshes_existing_block() -> Result<()> {
        let temp = TempDir::new()?;
        run_git(temp.path(), ["init"])?;
        let exclude_path = temp.path().join(".git").join("info").join("exclude");
        fs::write(
            &exclude_path,
            "# ply:start\n.ply/generated/\n# ply:end\ncustom-entry\n",
        )?;

        ensure_local_excludes(
            temp.path(),
            InitOptions {
                scaffold_local_packages: false,
                ignore_config: false,
            },
        )?;

        let content = fs::read_to_string(&exclude_path)?;
        assert!(content.contains("ply.local.toml"));
        assert!(content.contains("ply.ssh.toml"));
        assert!(content.contains("custom-entry"));
        Ok(())
    }

    #[test]
    fn resolve_github_shorthand() -> Result<()> {
        let temp = TempDir::new()?;
        match resolve_repo_spec(temp.path(), "owner/repo")? {
            ResolvedRepoSpec::GitHubShorthand(slug) => assert_eq!(slug, "owner/repo"),
            other => panic!("expected github shorthand, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn resolve_existing_relative_path_as_local_repo() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("owner").join("repo"))?;
        match resolve_repo_spec(temp.path(), "owner/repo")? {
            ResolvedRepoSpec::LocalPath(path) => {
                assert_eq!(path, temp.path().join("owner").join("repo").canonicalize()?)
            }
            other => panic!("expected local path, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn build_ssh_command_from_key_path() -> Result<()> {
        let command = ssh_command(Some(&SshSourceConfig {
            use_ssh: true,
            ssh_key_path: Some("~/.ssh/id_test".to_string()),
            ssh_key_env: None,
        }))?;
        assert!(command.unwrap().contains("IdentitiesOnly=yes"));
        Ok(())
    }
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
