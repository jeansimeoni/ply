# Consume Packages In a Project Repo

This guide is for a repository that wants to consume reusable Ply packages. It
covers the normal project workflow:

1. Initialize Ply in the repo.
2. Add one or more package sources.
3. Apply managed assets.
4. Verify the result.

Examples below assume the `ply` binary is already installed and available on
your `PATH`.

## Before you start

- The target directory must be a Git repository.
- Ply does not take ownership of repository-managed files such as `AGENTS.md`
  or `CLAUDE.md`.
- Ply writes its generated state under `.ply/` and exposes managed assets
  into adapter-owned locations such as `.claude/`, `.codex/`, and `.agents/`.

## Initialize the project

Run:

```bash
ply init
```

This prepares the local Ply state for the repo:

- `ply.toml`
- `ply.local.toml`
- `.ply/state.json`
- `.ply/generated/`
- default local overlay directories under `.ply/overlays/`
- a clone-local ignore block in `.git/info/exclude`

By default, `ply init` also scaffolds a local example package source at
`ply-packages/example-review`.

Useful options:

- `--with-packages` creates the example local package source explicitly.
- `--without-packages` skips the example local package source.
- `--ignore-config` keeps Ply config ignored locally in this clone. This is
  the default.
- `--track-config` leaves `ply.toml`, `ply.lock`, and `ply-packages/`
  trackable.
- `--dry-run` previews what would be created.

## Understand the starting manifest

A default `ply.toml` looks like this:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = true

[[sources]]
id = "local"
kind = "path"
path = "./ply-packages/example-review"
```

Important points:

- `schema_version = 1` is the only supported schema version.
- `mode = "copy"` is the only implemented install mode.
- `use_global = true` means project composition includes the user-global Ply
  layer if one exists.
- Each `[[sources]]` entry points at one package root that must contain
  `ply-package.toml`.

## Add package sources

Ply supports `path` and `git` sources.

Normal workflow:

```bash
ply add --id local --path ./ply-packages/example-review
ply add --id team --git owner/ply-team --rev main
```

The examples below show the equivalent manifest shape written into
`ply.toml`.

Local path source:

```toml
[[sources]]
id = "team"
kind = "path"
path = "../ply-team"
```

Git source using GitHub shorthand:

```toml
[[sources]]
id = "team"
kind = "git"
repo = "owner/ply-team"
rev = "main"
```

Git source using a full remote:

```toml
[[sources]]
id = "team"
kind = "git"
repo = "git@github.com:owner/ply-team.git"
rev = "v1.2.0"
```

`repo` accepts:

- a local path
- GitHub shorthand such as `owner/repo`
- a full `https://`, `ssh://`, or `git@` remote

`rev` accepts:

- a branch name
- a tag
- a commit SHA

For Git sources that point at a local repo path, Ply currently supports
`rev = "HEAD"` or no `rev`.

To remove a source later:

```bash
ply remove team
```

## Apply managed assets

Preview the composition first:

```bash
ply apply --dry-run
```

The dry-run shows:

- which sources and packages resolved
- which overlays apply
- which managed assets would be written
- whether any existing managed files have drifted

Then write the managed assets:

```bash
ply apply
```

If Ply finds drift in files it already manages, it asks before overwriting
those exposed files. Use `ply apply --yes` only when you want Ply to accept
all overwrite prompts automatically.

Successful `apply` also updates:

- `ply.lock` with resolved source revisions
- `.ply/state.json` with the set of files Ply currently owns
- `.ply/generated/` with deterministic generated outputs

To refresh Git source revisions without applying managed assets:

```bash
ply update
ply update team
```

## Verify the result

Check the current package set:

```bash
ply list
```

Inspect resolved source revisions:

```bash
ply sources
```

Check drift against the current desired state:

```bash
ply diff
```

Validate manifest, source resolution, ignore coverage, and state drift:

```bash
ply doctor
```

## Local-only overrides

Use `ply.local.toml` for machine-local changes that should not affect the
shared project manifest.

Common uses:

- point a shared Git source at a local checkout
- change `rev` locally
- add overlays that only apply in one clone

Example:

```toml
schema_version = 1

[[sources]]
id = "team"
repo = "../ply-team"
rev = "HEAD"

[[overlays]]
adapter = "codex"
kind = "skills"
path = ".ply/overlays/codex/skills"
```

## What Ply does not do

- It does not edit repository-owned `AGENTS.md` or `CLAUDE.md`.
- It does not modify `.gitignore` by default.
- It does not support install modes other than `copy`.
- It does not treat one source as multiple packages. Each source points at one
  package root.

## Next steps

- If you want reusable personal defaults across repos, continue with
  [Use a global Ply layer](../guides/global-layer.md).
- If you need package maintenance commands, see
  [Inspect, update, and clean managed assets](../guides/manage-managed-assets.md).
- If you need schema details, see
  [Configuration and layering](../reference/configuration-and-layering.md).
