# ply

Composable package manager for coding-agent assets across Claude Code, Codex,
and other AI developer tools.

Ply is local-first and additive. It installs reusable agent assets without
taking ownership of repository-managed instruction files such as `AGENTS.md` or
`CLAUDE.md`.

## Development

This repository uses [`mise`](https://mise.jdx.dev/) to pin the Rust toolchain.

```bash
mise install
cargo test
```

If you prefer not to activate `mise` in your shell, run commands through it
directly:

```bash
mise exec -- cargo test
```

## MVP status

The current MVP supports:

- `ply init`
- `ply init package`
- `ply apply`
- `ply diff`
- `ply doctor`
- `ply list`
- `ply sources`
- `ply adapters`
- `ply clean` / `ply nuke`

Implemented behavior:

- local path sources
- Git sources with:
  - `repo` as local path, GitHub shorthand, or full remote
  - local semantic overrides through `ply.local.toml`
  - local SSH transport config through `ply.ssh.toml`
  - pinned revisions in `ply.lock`
- deterministic generation under `.ply/generated/`
- Claude and Codex asset mapping for:
  - `commands`
  - `skills`
  - `agents`
  - `local-instructions`
  - `rules`
  - `hooks`
  - `output-styles`
- managed-block updates for `CLAUDE.local.md`
- generated local composite output for `AGENTS.override.md`
- Codex hook registration through `.codex/hooks.json`
- grouped drift and safety reporting in `ply diff`
- validation of ignore coverage and state drift in `ply doctor`
- local-only Git ignore management via `.git/info/exclude`
- tracked-file and unmanaged-file collision checks
- destructive cleanup flow with confirmation
- optional user-global layering from `~/.config/ply`
- dry-run support for `init`, `apply`, and `clean`

## Design constraints

Ply is designed to coexist with repository-owned agent context.

- Ply does not take ownership of repository-managed `AGENTS.md` or
  `CLAUDE.md`
- Ply prefers additive, local-only behavior over destructive replacement
- Ply uses `.git/info/exclude` for clone-local ignore rules by default instead
  of modifying `.gitignore`
- managed assets should use the `ply-` prefix when exposed into adapter-owned
  namespaces

## Core workflows

Ply manages two related but different workflows:

- consuming shared agent resources in a project repo through `ply init` and
  `ply apply`
- authoring reusable package content through `ply init package`

`ply init package` is intended for any directory that will become a package
root, including a standalone folder or a dedicated Git repository.

## Project manifest

`ply.toml`:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"

[[sources]]
id = "team"
kind = "git"
repo = "owner/ply-team"
rev = "main"

[[packages]]
source = "team"
path = "."
```

Git source `repo` accepts:

- local path: `../ply-team`
- GitHub shorthand: `owner/ply-team`
- full remote: `https://...`, `ssh://...`, or `git@...`

Git source `rev` accepts:

- a branch name such as `main` or `master`
- a tag
- a commit SHA

## Local config layers

Shared project intent lives in `ply.toml`.

`ply.local.toml` is optional and local-only. Use it to override or add
sources, packages, and overlays on one machine without changing the shared
project manifest.

Example:

```toml
[[sources]]
id = "team"
kind = "git"
repo = "../ply-team"
rev = "HEAD"

[[overlays]]
adapter = "codex"
kind = "skills"
path = ".ply/overlays/codex/skills"
```

`ply.ssh.toml` is also optional and local-only. Use it for source-specific
SSH transport preferences and keys.

Example:

```toml
[sources.team]
use_ssh = true
ssh_key_path = "~/.ssh/id_ply_team"
```

With that combination, a shared shorthand source such as
`repo = "owner/ply-team"` can be used over GitHub SSH locally without
modifying `ply.toml`.

## Package layout

Each package contains a `ply-package.toml` file plus shared top-level asset
kinds:

```txt
example-review/
├── ply-package.toml
├── commands/
├── agents/
├── hooks/
├── local-instructions.md
├── output-styles/
├── rules/
└── skills/
    └── ply-review-diff/
        └── SKILL.md
```

Managed assets must use the `ply-` prefix at the top level, for example:

- `skills/ply-review-diff/`
- `agents/ply-reviewer/`
- `commands/ply-pr-review.md`

## Per-resource adapter targeting

Package resources target all adapters enabled in the consuming project's
`ply.toml` by default.

To limit a resource to selected adapters, add metadata with a `targets` list:

- directory resources such as `skills/ply-review-diff/` use
  `skills/ply-review-diff/ply-asset.toml`
- file resources such as `commands/ply-pr-review.md` use a sidecar file like
  `commands/ply-pr-review.md.ply-asset.toml`

This is especially useful for adapter-specific kinds such as `agents`, which
currently map only to Claude.

Example directory metadata:

```toml
targets = ["claude"]
```

Example file metadata:

```toml
targets = ["codex"]
```

If `targets` is omitted or empty, the resource applies to all enabled adapters.

## Adapter mapping

Current MVP adapter mapping:

Claude:

- `commands` -> `.claude/commands/`
- `skills` -> `.claude/skills/`
- `agents` -> `.claude/agents/`
- `local-instructions` -> `CLAUDE.local.md`
- `rules` -> `.claude/rules/`
- `hooks` -> `.claude/hooks/`
- `output-styles` -> `.claude/output-styles/`

Codex:

- `commands` -> `.agents/commands/`
- `skills` -> `.agents/skills/`
- `local-instructions` -> `AGENTS.override.md`
- `rules` -> `.codex/rules/`
- `hooks` -> `.codex/hooks/` plus `.codex/hooks.json`
- `output-styles` -> `AGENTS.override.md`

Shared package content is exposed according to the consuming project's
`ply.toml` adapters. The same package `skills/` or `commands/` content can be
applied into both Claude and Codex when both adapters are enabled in the
project. Adapter-specific kinds such as `agents` are exposed only to adapters
that support them.

Ply is intentionally conservative around repository-owned files:

- Ply never edits repository-owned `CLAUDE.md` or `AGENTS.md`.
- `CLAUDE.local.md` is updated only inside a Ply-managed block.
- `AGENTS.override.md` is fully generated by Ply and may include the current
  `AGENTS.md` content followed by Ply-managed local sections.

## Local overlays

Local overlays are configured in `ply.local.toml` and applied after package
composition:

```toml
[[overlays]]
adapter = "codex"
kind = "skills"
path = ".ply/overlays/codex/skills"

[[overlays]]
adapter = "claude"
kind = "local-instructions"
path = ".ply/overlays/claude/local-instructions.md"
```

Overlays follow the same adapter and asset-kind structure as exposed assets,
but they remain adapter-specific because they target the local consuming
project surfaces directly.

For compatibility, legacy `.ply/local.yml` overlays are still loaded when
present.

## Git ignore policy

Ply should manage clone-local ignore behavior through `.git/info/exclude`
rather than `.gitignore` by default.

Ply-managed paths that typically need ignore coverage include:

- `.ply/generated/`
- `.ply/state.json`
- `ply.local.toml`
- `ply.ssh.toml`
- `AGENTS.override.md`
- `CLAUDE.local.md`
- any Ply-managed `.claude/*`, `.codex/*`, or `.agents/*` assets

## Global layer

User-global Ply config lives under `~/.config/ply` and is layered into
project composition by default. Projects can opt out with `use_global = false`
under `[install]`.
