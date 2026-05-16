# ply

Composable package manager for coding-agent assets across Claude Code, Codex, and other AI developer tools.

## Development

This repository uses [`mise`](https://mise.jdx.dev/) to pin the Rust toolchain.

```bash
mise install
cargo test
```

If you prefer not to activate `mise` in your shell, run commands through it directly:

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

Each package contains a `ply-package.toml` file plus adapter-specific asset trees:

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

## Local overlays

Local overlays are configured in `.ply/local.yml` and applied after package composition:

```yaml
overlays:
  - adapter: codex
    kind: skills
    path: .ply/overlays/codex/skills
```
