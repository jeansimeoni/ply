# Use a Global Ply Layer

This guide covers the optional user-global Ply layer under `~/.config/ply`.

Use the global layer when you want personal packages or overlays to follow you
across many repos without committing those choices into each project.

## What the global layer is

The global layer is a normal Ply root stored at:

```txt
~/.config/ply
```

When a project has this in `ply.toml`:

```toml
[install]
use_global = true
```

Ply loads the global layer first, then layers the project on top of it.

Practical effect:

- global sources and overlays can provide shared defaults
- project sources and overlays can override or add to them
- if the same source `id` exists in both layers, the project definition wins

## Initialize the global root

Run:

```bash
ply init --global
```

or:

```bash
ply init -g
```

This creates the same basic Ply structure used in a project root, but under
`~/.config/ply`.

Unlike project mode, global mode does not require a Git repository.

## Add global sources

Use `ply add -g` from any directory or edit `ply.toml` directly to add the
sources you want available to all participating projects.

Example command:

```bash
ply add -g --id personal --path /home/you/agent-packages/review-tools
ply add -g --id team --git owner/repo --rev main --ssh
```

Example:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true

[[sources]]
id = "personal"
kind = "path"
path = "/home/you/agent-packages/review-tools"
```

Use global sources for:

- personal rules or skills you want in many repos
- local private packages that should not live in each project manifest
- a stable base layer that projects can selectively override

Keep a source in project config instead when it is specific to one repository,
team, or codebase.

## Add global overlays

The global root also supports `ply.local.toml` overlays.

Example:

```toml
schema_version = 1

[[overlays]]
adapter = "codex"
kind = "skills"
path = ".ply/overlays/codex/skills"
```

Global overlays are loaded before project overlays. They are useful for personal
local additions that should never be committed into a team repo.

## Verify the global root

Inspect the global package list:

```bash
ply list -g
```

Inspect the global source set:

```bash
ply sources -g
```

Validate the global root:

```bash
ply doctor -g
```

These commands inspect the global root itself. A project that has
`use_global = true` still needs its own `ply apply` run inside the repo to
compose global and project layers together.

## Opt out in a project

If a project should ignore the user-global layer, set:

```toml
[install]
use_global = false
```

That project will compose only its own manifest, local overrides, and local
overlays.

## Clean the global root

Preview cleanup:

```bash
ply clean -g --dry-run
```

Remove Ply-managed global files:

```bash
ply clean -g
```

This removes Ply-managed files and local state for the global root. It does
not delete unrelated files in `~/.config/ply`.

## Next steps

- For project setup, see
  [Consume packages in a project repo](consume-packages-project.md).
- For config details, see
  [Configuration and layering](../reference/configuration-and-layering.md).
