use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::hash_map::DefaultHasher, fmt::Write as _};

use crate::config::{InitOptions, SourceConfig, SshSourceConfig};
use crate::ui;

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
.codex/agents/ply-*.toml
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
    let repo = source_repo(source)?;
    let spec = resolve_repo_spec(project_root, repo)?;
    let repo_path = source_checkout_root(project_root, source)?;
    let remote = remote_from_repo_spec(&spec, ssh_config);

    ensure_repo_cloned(project_root, source, &repo_path, &remote, ssh_config)?;
    refresh_existing_repo(source, &repo_path, ssh_config)?;

    let rev = source.rev.as_deref().unwrap_or("HEAD");
    let resolved = resolve_checkout_revision(&repo_path, rev, ssh_command(ssh_config)?.as_deref())?;
    checkout_revision(source, &repo_path, ssh_config, resolved.trim())?;
    Ok((repo_path, resolved.trim().to_string()))
}

pub fn ensure_source_at_revision(
    project_root: &Path,
    source: &SourceConfig,
    ssh_config: Option<&SshSourceConfig>,
    revision: &str,
) -> Result<PathBuf> {
    let repo = source_repo(source)?;
    let spec = resolve_repo_spec(project_root, repo)?;
    let repo_path = source_checkout_root(project_root, source)?;
    let remote = remote_from_repo_spec(&spec, ssh_config);

    ensure_repo_cloned(project_root, source, &repo_path, &remote, ssh_config)?;
    checkout_revision(source, &repo_path, ssh_config, revision)?;
    Ok(repo_path)
}

pub fn source_checkout_root(project_root: &Path, source: &SourceConfig) -> Result<PathBuf> {
    let repo = source_repo(source)?;
    let identity = repo_identity(project_root, repo)?;
    let cache_root = project_root.join(".ply").join("cache").join("sources");
    fs::create_dir_all(&cache_root)?;
    Ok(cache_root.join(identity))
}

#[derive(Debug)]
enum ResolvedRepoSpec {
    LocalPath(PathBuf),
    GitHubShorthand(String),
    Remote(String),
}

fn source_repo(source: &SourceConfig) -> Result<&str> {
    source
        .repo
        .as_deref()
        .or(source.url.as_deref())
        .ok_or_else(|| anyhow!("git source `{}` missing repo", source.id))
}

fn resolve_repo_spec(project_root: &Path, repo: &str) -> Result<ResolvedRepoSpec> {
    if repo.starts_with("git@") || repo.starts_with("ssh://") || repo.contains("://") {
        return Ok(ResolvedRepoSpec::Remote(repo.to_string()));
    }

    let repo_path = PathBuf::from(repo);
    if repo_path.is_absolute() || repo.starts_with("./") || repo.starts_with("../") {
        return Ok(ResolvedRepoSpec::LocalPath(
            project_root.join(repo).canonicalize().with_context(|| {
                format!(
                    "failed to resolve local repo path {}",
                    project_root.join(repo).display()
                )
            })?,
        ));
    }

    let candidate = project_root.join(repo);
    if candidate.exists() {
        return Ok(ResolvedRepoSpec::LocalPath(
            candidate.canonicalize().with_context(|| {
                format!("failed to resolve local repo path {}", candidate.display())
            })?,
        ));
    }

    if repo.matches('/').count() == 1 {
        return Ok(ResolvedRepoSpec::GitHubShorthand(repo.to_string()));
    }

    Err(anyhow!("unsupported git repo spec `{repo}`"))
}

fn remote_from_repo_spec(spec: &ResolvedRepoSpec, ssh_config: Option<&SshSourceConfig>) -> String {
    match spec {
        ResolvedRepoSpec::LocalPath(path) => path.display().to_string(),
        ResolvedRepoSpec::GitHubShorthand(slug) => {
            if ssh_config.map(|config| config.use_ssh).unwrap_or(false) {
                format!("git@github.com:{slug}.git")
            } else {
                format!("https://github.com/{slug}")
            }
        }
        ResolvedRepoSpec::Remote(remote) => remote.clone(),
    }
}

fn repo_identity(project_root: &Path, repo: &str) -> Result<String> {
    let spec = resolve_repo_spec(project_root, repo)?;
    let raw_identity = match spec {
        ResolvedRepoSpec::LocalPath(path) => format!("local:{}", path.display()),
        ResolvedRepoSpec::GitHubShorthand(slug) => format!("github:{slug}"),
        ResolvedRepoSpec::Remote(remote) => format!("remote:{remote}"),
    };
    let mut hasher = DefaultHasher::new();
    raw_identity.hash(&mut hasher);
    let mut suffix = String::new();
    let _ = write!(&mut suffix, "{:016x}", hasher.finish());
    Ok(format!("source-{suffix}"))
}

fn ensure_repo_cloned(
    project_root: &Path,
    source: &SourceConfig,
    repo_path: &Path,
    remote: &str,
    ssh_config: Option<&SshSourceConfig>,
) -> Result<()> {
    if repo_path.exists() {
        return Ok(());
    }
    let ssh_command = ssh_command(ssh_config)?;
    let progress = ui::start_progress(&format!("Cloning Git source `{}`", source.id));
    match run_git_args_with_env(
        project_root,
        ["clone", "--quiet", remote],
        Some(repo_path.as_os_str()),
        ssh_command.as_deref(),
        "git clone",
    ) {
        Ok(()) => {
            progress.success();
            Ok(())
        }
        Err(err) => {
            progress.error();
            Err(err).with_context(|| format!("failed to clone source `{}`", source.id))
        }
    }
}

fn refresh_existing_repo(
    source: &SourceConfig,
    repo_path: &Path,
    ssh_config: Option<&SshSourceConfig>,
) -> Result<()> {
    let ssh_command = ssh_command(ssh_config)?;
    let progress = ui::start_progress(&format!("Fetching Git source `{}`", source.id));
    match run_git_args_with_env(
        repo_path,
        ["fetch", "--all", "--tags", "--prune", "--quiet"],
        None,
        ssh_command.as_deref(),
        "git fetch",
    ) {
        Ok(()) => {
            progress.success();
            Ok(())
        }
        Err(err) => {
            progress.error();
            Err(err).with_context(|| format!("failed to refresh source `{}`", source.id))
        }
    }
}

fn resolve_checkout_revision(
    repo_path: &Path,
    rev: &str,
    ssh_command: Option<&str>,
) -> Result<String> {
    if rev == "HEAD"
        && let Ok(resolved) = run_git_with_env(
            repo_path,
            ["rev-parse", "--verify", "refs/remotes/origin/HEAD"],
            ssh_command,
        )
    {
        return Ok(resolved);
    }
    if rev != "HEAD" {
        let remote_ref = format!("refs/remotes/origin/{rev}");
        if let Ok(resolved) = run_git_with_env(
            repo_path,
            ["rev-parse", "--verify", &remote_ref],
            ssh_command,
        ) {
            return Ok(resolved);
        }
    }
    run_git_with_env(repo_path, ["rev-parse", rev], ssh_command)
}

fn checkout_revision(
    source: &SourceConfig,
    repo_path: &Path,
    ssh_config: Option<&SshSourceConfig>,
    revision: &str,
) -> Result<()> {
    let ssh_command = ssh_command(ssh_config)?;
    let progress = ui::start_progress(&format!(
        "Checking out Git source `{}` at {}",
        source.id, revision
    ));
    match run_git_args_with_env(
        repo_path,
        [
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--force",
            revision,
        ],
        None,
        ssh_command.as_deref(),
        "git checkout",
    ) {
        Ok(()) => {
            progress.success();
            Ok(())
        }
        Err(err) => {
            progress.error();
            Err(err).with_context(|| {
                format!(
                    "failed to checkout revision `{}` for source `{}`",
                    revision, source.id
                )
            })
        }
    }
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

pub fn run_git<I, S>(project_root: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_git_with_env(project_root, args, None)
}

fn run_git_with_env<I, S>(project_root: &Path, args: I, ssh_command: Option<&str>) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.args(args).current_dir(project_root);
    if let Some(ssh_command) = ssh_command {
        command.env("GIT_SSH_COMMAND", ssh_command);
    }
    let output = command.output().context("failed to run git command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_args_with_env<I, S>(
    project_root: &Path,
    args: I,
    trailing_arg: Option<&OsStr>,
    ssh_command: Option<&str>,
    label: &str,
) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.args(args).current_dir(project_root);
    if let Some(trailing_arg) = trailing_arg {
        command.arg(trailing_arg);
    }
    if let Some(ssh_command) = ssh_command {
        command.env("GIT_SSH_COMMAND", ssh_command);
    }
    let output = command
        .output()
        .with_context(|| format!("failed to run `{label}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "git exited with a non-zero status".to_string()
        };
        return Err(anyhow!("{label} failed: {detail}"));
    }
    Ok(())
}

fn relative_path<'a>(project_root: &'a Path, path: &'a Path) -> Result<&'a Path> {
    path.strip_prefix(project_root)
        .with_context(|| format!("{} is not under {}", path.display(), project_root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn commit_file(repo: &Path, name: &str, content: &str) -> Result<String> {
        fs::write(repo.join(name), content)?;
        run_git(repo, ["add", name])?;
        let _ = run_git(repo, ["commit", "-m", "update fixture"])?;
        run_git(repo, ["rev-parse", "HEAD"])
    }

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
                adapters: &["codex", "claude"],
            },
        )?;

        let content = fs::read_to_string(&exclude_path)?;
        assert!(content.contains("ply.local.toml"));
        assert!(content.contains("ply.ssh.toml"));
        assert!(content.contains("custom-entry"));
        Ok(())
    }

    #[test]
    fn ensure_local_excludes_adds_config_files_when_ignored() -> Result<()> {
        let temp = TempDir::new()?;
        run_git(temp.path(), ["init"])?;

        ensure_local_excludes(
            temp.path(),
            InitOptions {
                scaffold_local_packages: false,
                ignore_config: true,
                adapters: &["codex", "claude"],
            },
        )?;

        let exclude_path = temp.path().join(".git").join("info").join("exclude");
        let content = fs::read_to_string(&exclude_path)?;
        assert!(content.contains("ply.toml"));
        assert!(content.contains("ply.lock"));
        assert!(content.contains("ply-packages/"));
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

    #[test]
    fn clone_or_update_source_refreshes_branch_tip() -> Result<()> {
        let temp = TempDir::new()?;
        let remote = temp.path().join("remote.git");
        run_git(temp.path(), ["init", "--bare", remote.to_str().unwrap()])?;
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(&worktree)?;
        run_git(&worktree, ["init", "-b", "master"])?;
        run_git(&worktree, ["config", "user.email", "test@example.com"])?;
        run_git(&worktree, ["config", "user.name", "Test User"])?;
        run_git(
            &worktree,
            ["remote", "add", "origin", remote.to_str().unwrap()],
        )?;
        let first = commit_file(&worktree, "ply-package.toml", "name = \"fixture\"\n")?;
        run_git(&worktree, ["push", "origin", "master"])?;

        let source = SourceConfig {
            id: "fixture".to_string(),
            kind: "git".to_string(),
            path: None,
            repo: Some(format!("file://{}", remote.display())),
            url: None,
            rev: Some("master".to_string()),
        };

        let (_, resolved_first) = clone_or_update_source(temp.path(), &source, None)?;
        assert_eq!(resolved_first, first);

        let second = commit_file(&worktree, "README.md", "# updated\n")?;
        run_git(&worktree, ["push", "origin", "master"])?;
        let (_, resolved_second) = clone_or_update_source(temp.path(), &source, None)?;
        assert_eq!(resolved_second, second);
        Ok(())
    }

    #[test]
    fn source_checkout_root_is_stable_across_source_id_renames() -> Result<()> {
        let temp = TempDir::new()?;
        let first = SourceConfig {
            id: "one".to_string(),
            kind: "git".to_string(),
            path: None,
            repo: Some("owner/repo".to_string()),
            url: None,
            rev: Some("HEAD".to_string()),
        };
        let second = SourceConfig {
            id: "two".to_string(),
            kind: "git".to_string(),
            path: None,
            repo: Some("owner/repo".to_string()),
            url: None,
            rev: Some("HEAD".to_string()),
        };

        assert_eq!(
            source_checkout_root(temp.path(), &first)?,
            source_checkout_root(temp.path(), &second)?,
        );
        Ok(())
    }
}
