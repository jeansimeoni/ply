# Configuration And Layering

This reference explains the configuration files Ply reads and how they
compose.

## Config files

### `ply.toml`

The shared manifest for a project root or the global root.

Example:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true

[[sources]]
id = "team"
kind = "git"
repo = "owner/ply-team"
rev = "main"
```

Important fields:

- `schema_version`: only `1` is supported
- `adapters`: defaults to `["codex", "claude"]`, or whatever subset was chosen during `ply init --adapters`
- `[install].mode`: only `"copy"` is implemented
- `[install].use_global`: defaults to `true`
- `[[sources]]`: the package roots to consume
- each source points at exactly one package root

### `ply.local.toml`

Optional local-only manifest extensions for one machine or clone.

It can:

- override existing sources by `id`
- add new local-only sources
- add local overlays

For an existing source `id`, local values override the shared source fields
that are present in `ply.local.toml`.

If a local source `id` does not already exist, the local entry must define
`kind`.

### `ply.ssh.toml`

Optional local-only SSH transport settings for Git sources.

Example:

```toml
[sources.team]
use_ssh = true
ssh_key_path = "~/.ssh/id_ply_team"
```

Supported fields:

- `use_ssh`
- `ssh_key_path`
- `ssh_key_env`

### `ply.lock`

Written by `ply apply` and `ply update`.

This lockfile records the resolved source identifier, source kind, and resolved
revision that was applied.

### `.ply/state.json`

Written by `ply init` and updated by `ply apply`.

This local state file records:

- the install mode
- whether init chose local ignore behavior for config files
- the set of managed paths Ply currently owns

Ply uses this state to reason about drift, managed ownership, and stale path
cleanup.

## Source kinds

Supported source kinds:

- `path`
- `git`

`path` sources require `path`.

`git` sources require either `repo` or legacy `url`, but not both.

`repo` supports:

- local paths
- GitHub shorthand such as `owner/repo`
- full remote URLs

## Global and project layering

When a project has `use_global = true`, Ply composes layers in this order:

1. global root at `~/.config/ply`, if initialized
2. the current project root

Project values layer on top of global values.

Practical consequences:

- project source definitions can override global sources with the same `id`
- project-only sources are added after global sources
- adapters are deduplicated across layers
- overlays from both layers remain active

Set `use_global = false` in a project to disable global composition for that
repo.

## Overlay loading

Overlays are local-only and are applied after package composition.

Ply loads overlays from:

- `ply.local.toml`
- legacy `.ply/local.yml`, if present

Default overlay scaffolding points at:

- `.ply/overlays/codex/skills`
- `.ply/overlays/claude/skills`

Overlay entries require:

- `adapter`
- `kind`
- `path`

For generated composite targets such as Markdown-based local instructions, the
overlay path must point to a Markdown file.

## Local ignore behavior

When the target is a Git repo, `ply init` updates `.git/info/exclude` instead
of `.gitignore`.

The managed ignore block always covers local Ply state such as:

- `.ply/generated/`
- `.ply/cache/`
- `.ply/state.json`
- `ply.local.toml`
- `ply.ssh.toml`
- managed adapter outputs

If init runs with `--ignore-config` or the default prompt choice, the managed
ignore block also covers:

- `.ply/`
- `ply.toml`
- `ply.lock`
- `ply-packages/`

If init runs with `--track-config`, those shared config files remain trackable.

## Ownership boundaries

Generated or managed outputs belong to Ply.

Repository-owned files do not.

Examples of repository-owned files Ply avoids taking over:

- `AGENTS.md`
- `CLAUDE.md`

Examples of Ply-managed outputs:

- `.claude/commands/ply-*.md`
- `.claude/skills/ply-*/`
- `.agents/commands/ply-*.md`
- `.agents/skills/ply-*/`
- `.codex/agents/ply-*.toml`
- `CLAUDE.local.md` managed block content
- `AGENTS.override.md`

## Related docs

- [Consume packages in a project repo](../guides/consume-packages-project.md)
- [Use a global Ply layer](../guides/global-layer.md)
- [CLI command reference](cli.md)
