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
- shared frontmatter-based authoring for prompt resources:
  - `commands`
  - `skills`
  - `agents`
  - `output-styles`
- managed-block updates for `CLAUDE.local.md`
- generated local composite output for `AGENTS.override.md`
- Codex hook registration through `.codex/hooks.json`
- generated Codex agent `.toml` files and Codex skill `agents/openai.yaml` sidecars
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
- managed assets are exposed under the `ply-` prefix in adapter-owned
  namespaces

## Core workflows

Ply manages two related but different workflows:

- consuming shared agent resources in a project repo through `ply init` and
  `ply apply`
- authoring one reusable package root through `ply init package`

`ply init package` is intended for any directory that will become a package
root, including a standalone folder or a dedicated Git repository. Each
configured source points directly at one package root.

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
```

Each configured source is treated as one package root. Ply expects
`ply-package.toml` at that source root.

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
sources and overlays on one machine without changing the shared project
manifest.

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
│   └── reviewer/
│       └── AGENT.md
├── hooks/
├── local-instructions.md
├── output-styles/
├── rules/
└── skills/
    └── review-diff/
        └── SKILL.md
```

Package authors can use natural top-level names. Ply prefixes managed exposed
assets with `ply-` automatically when writing into adapter-owned namespaces.

Examples:

- `skills/review-diff/` is exposed as `.claude/skills/ply-review-diff/`
- `agents/reviewer/` is exposed as `.claude/agents/ply-reviewer/`
- `commands/pr-review.md` is exposed as `.claude/commands/ply-pr-review.md`

`agents/` uses shared Markdown authoring. Each agent resource lives in its own
directory and provides `AGENT.md` as the instruction source document.

`skills/`, `commands/`, `agents/`, and `output-styles/` are prompt resources.
They can use shared YAML frontmatter plus adapter-specific sections. `rules/`
and `hooks/` remain native adapter resources and are not part of this
frontmatter system.

Example:

```md
---
name: technical-writer
description: Write clear technical documentation

claude:
  tools:
    - Read
    - Write

codex:
  model: gpt-5.5
  reasoning_effort: medium
---

Write accurate documentation with verifiable examples.
```

## Per-resource adapter targeting

Package resources target all adapters enabled in the consuming project's
`ply.toml` by default.

To limit a resource to selected adapters, add metadata with a `targets` list:

- directory resources such as `skills/review-diff/` use
  `skills/review-diff/ply-asset.toml`
- file resources such as `commands/pr-review.md` use a sidecar file like
  `commands/pr-review.md.ply-asset.toml`

This is especially useful for adapter-specific kinds such as `agents`, which
render differently per adapter.

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
- `agents` -> generated `.codex/agents/*.toml`
- `local-instructions` -> `AGENTS.override.md`
- `rules` -> `.codex/rules/`
- `hooks` -> `.codex/hooks/` plus `.codex/hooks.json`
- `output-styles` -> `AGENTS.override.md`

Shared package content is exposed according to the consuming project's
`ply.toml` adapters. The same package `skills/` or `commands/` content can be
applied into both Claude and Codex when both adapters are enabled in the
project.

For `agents`, Ply keeps package authoring shared and renders adapter-native
outputs:

- Claude receives the authored `agents/<name>/` directory directly
- Codex receives a generated `.codex/agents/<name>.toml` file with `name`,
  `description`, and `developer_instructions` from `AGENT.md`

For `skills`, Ply keeps `SKILL.md` and companion directories shared while
also rendering Codex-native skill metadata when needed:

- Claude receives the authored `SKILL.md` plus companion files
- Codex receives a generated `SKILL.md` with required YAML frontmatter plus the
  companion files
- Codex also receives generated `agents/openai.yaml` when Codex skill metadata
  is present in frontmatter

For `commands` and Codex `output-styles`, Codex-specific metadata is translated
into a deterministic generated settings preamble inside the exposed Markdown or
generated `AGENTS.override.md` sections.

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
