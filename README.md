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
- `ply apply`
- `ply diff`
- `ply doctor`
- `ply list`
- `ply sources`
- `ply adapters`
- `ply clean` / `ply nuke`

Implemented behavior:

- local path sources
- Git sources pinned in `ply.lock`
- deterministic generation under `.ply/generated/`
- safe copy-mode exposure for Codex and Claude assets
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

## Project manifest

`ply.toml`:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"

[[sources]]
id = "local"
kind = "path"
path = "./ply-packages"

[[packages]]
source = "local"
path = "example-review"
```

## Package layout

Each package contains a `ply-package.toml` file plus adapter-specific asset
trees:

```txt
example-review/
├── ply-package.toml
├── codex/
│   ├── commands/
│   └── skills/
└── claude/
    ├── commands/
    └── skills/
```

Managed assets must use the `ply-` prefix at the top level, for example:

- `codex/skills/ply-review-diff/`
- `claude/commands/ply-pr-review.md`

## Adapter mapping

Current MVP adapter mapping:

Claude:

- `commands` -> `.claude/commands/`
- `skills` -> `.claude/skills/`

Codex:

- `commands` -> `.agents/commands/`
- `skills` -> `.agents/skills/`

Ply is intentionally conservative around repository-owned files:

- Ply never edits repository-owned `CLAUDE.md` or `AGENTS.md`.

## Local overlays

Local overlays are configured in `.ply/local.yml` and applied after package
composition:

```yaml
overlays:
  - adapter: codex
    kind: skills
    path: .ply/overlays/codex/skills
```

Overlays currently follow the same adapter and asset-kind structure as package
assets.

## Git ignore policy

Ply should manage clone-local ignore behavior through `.git/info/exclude`
rather than `.gitignore` by default.

Ply-managed paths that typically need ignore coverage include:

- `.ply/generated/`
- `.ply/state.json`
- `.ply/local.yml`
- any Ply-managed `.claude/*` or `.agents/*` assets

## Global layer

User-global Ply config lives under `~/.config/ply` and is layered into
project composition by default. Projects can opt out with `use_global = false`
under `[install]`.
