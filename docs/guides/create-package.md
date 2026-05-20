# Create Your First Package

This guide is for package authors who want to create a reusable Ply package
that other repos can consume.

## What a package is

A Ply package is one directory tree rooted at a single `ply-package.toml`
file.

Each configured source in a consuming project points at exactly one package
root. Ply does not select multiple packages from one source.

## Before you start

- Choose an empty directory or a directory that only contains bootstrap-safe
  files such as `.git`, `.gitignore`, `README.md`, or `LICENSE`.
- Make sure the package name you want to publish is available for
  `ply-package.toml`.
- Decide which top-level asset kinds you want to scaffold now. You can add more
  later.

## Initialize the package

Run:

```bash
ply init package --name review-tools --kinds skills,commands,agents
```

Important behavior:

- `--name` is required.
- `--path <dir>` is optional. If omitted, Ply uses the current directory.
- `--kinds <list>` is optional. If omitted, Ply creates only
  `ply-package.toml`.
- `--dry-run` previews the paths that would be created.

With the command above, Ply creates:

```txt
review-tools/
├── ply-package.toml
├── skills/
├── commands/
└── agents/
```

For file-based kinds such as `local-instructions`, Ply scaffolds an empty
`local-instructions.md` file instead of a directory.

## Package bootstrap safety rules

`ply init package` is intentionally conservative.

It refuses to initialize in a target directory when:

- the directory already contains unrelated files
- `ply-package.toml` already exists
- a requested target path such as `skills/` or `local-instructions.md` already
  exists

Allowed pre-existing bootstrap files include:

- `.git`
- `.gitignore`
- `README`
- `README.md`
- `LICENSE`
- `LICENSE.md`

This lets you bootstrap a package repo without overwriting existing content.

## Start with a minimal package

Create `ply-package.toml`:

```toml
name = "review-tools"
description = "Reusable review helpers"
```

Add a skill:

```txt
skills/
└── review-diff/
    └── SKILL.md
```

Example `skills/review-diff/SKILL.md`:

```md
---
name: review-diff
description: Review a diff with a bug-first mindset

claude:
  tools:
    - Read

codex:
  model: gpt-5.5
  reasoning_effort: medium
---

Review the diff for correctness issues, regressions, and missing tests.
```

## Test the package locally

The fastest feedback loop is to consume the package from a local path source in
another repo.

Consumer `ply.toml`:

```toml
schema_version = 1
adapters = ["codex", "claude"]

[install]
mode = "copy"
use_global = false

[[sources]]
id = "local-review-tools"
kind = "path"
path = "../review-tools"
```

Then in the consumer repo:

```bash
ply apply --dry-run
ply apply
ply diff
```

This verifies both package structure and rendered adapter output.

## Add adapter targeting when needed

Resources target all enabled adapters by default. Add a sidecar metadata file
only when a resource should apply to selected adapters.

Directory resource example:

```txt
skills/review-diff/
├── SKILL.md
└── ply-asset.toml
```

`ply-asset.toml`:

```toml
targets = ["claude"]
```

File resource example:

```txt
commands/pr-review.md
commands/pr-review.md.ply-asset.toml
```

## Authoring rules worth remembering

- Use natural package-local names. Ply adds the `ply-` prefix when it
  exposes managed assets into adapter-owned namespaces.
- `skills/`, `commands/`, `agents/`, and `output-styles/` are shared
  prompt-resource kinds and support YAML frontmatter.
- `rules/` and `hooks/` are native adapter resources and are not part of the
  frontmatter system.
- Do not author `agents/openai.yaml` directly inside a skill. Ply generates
  that Codex sidecar from supported Codex frontmatter metadata.

## Next steps

- For the full package schema, see
  [Package format and asset kinds](../reference/package-format.md).
- For source and layering behavior, see
  [Configuration and layering](../reference/configuration-and-layering.md).
