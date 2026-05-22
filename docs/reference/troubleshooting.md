# Troubleshooting

This page collects common failure modes in the current MVP.

## `ply is not initialized`

Symptom:

```txt
ply is not initialized in <path>; run `ply init` ...
```

Cause:

- `ply.toml` is missing for the current target root

Fix:

- run `ply init` in the project repo
- or run `ply init -g` for the user-global root

## Project commands fail outside a Git repo

Symptoms usually appear when running project-mode:

- `ply init`
- `ply apply`
- `ply clean`

Cause:

- the current directory is not a Git repository

Fix:

- run the command inside a Git repo
- or use `ply init -g`, `ply doctor -g`, `ply list -g`, and similar
  global-root commands when you intended to work with the global layer

## `source <id> is missing ply-package.toml at its root`

Cause:

- the configured source does not point at a valid package root

Fix:

- make sure the source path or repo root contains `ply-package.toml`
- if the source should point somewhere else, update `path` or `repo`
- re-run `ply sources` to confirm the resolved location

Remember that one source maps to one package root. Ply does not select a
sub-package from within a larger source tree.

## `package <name> contains unsupported adapter directory ...`

Cause:

- the package root contains adapter-owned directories such as `.claude/`,
  `.agents/`, or `.codex/`

Fix:

- move authored package content into portable package asset kinds such as
  `skills/`, `commands/`, `agents/`, `rules/`, `hooks/`, `output-styles/`, or
  `local-instructions.md`
- remove adapter-owned output directories from the package root

## `package <name> does not expose any supported managed assets`

Cause:

- the package root contains `ply-package.toml` but no supported managed
  assets

Fix:

- add at least one supported managed asset kind
- if you just ran `ply init package` without `--kinds`, add package content
  before consuming that package from a source
- or run `ply doctor package --fix` and answer the scaffold prompt

## `ply doctor package` reports a missing `ply-package.toml`

Cause:

- the package root has not been initialized yet

Fix:

- run `ply doctor package --fix`
- answer the prompts for the package name and any safe scaffolding choices

## Unsupported source or adapter

Common causes:

- a source `kind` other than `path` or `git`
- an adapter other than `codex` or `claude`
- an unsupported field in `ply.toml`, such as legacy `install.mode`

Fix:

- update the manifest to use only the currently documented schema

## `ply init package` refuses a non-empty directory

Symptoms include:

- `refusing to initialize package in non-empty directory`
- `ply-package.toml already exists`
- `target path <...> already exists`

Cause:

- the target directory contains unrelated files
- or the requested scaffold path already exists

Fix:

- use an empty directory
- or keep only bootstrap-safe files such as `.git`, `README.md`, or `LICENSE`
- or choose a different `--path`

## Frontmatter parse errors

Common causes:

- malformed YAML frontmatter
- unsupported keys in shared, `claude`, or `codex` sections
- using kind-specific metadata on the wrong resource kind

Examples:

- `argument-hint` outside `commands`
- `keep-coding-instructions` outside `output-styles`
- Codex skill sidecar fields outside `skills`
- Codex agent runtime fields outside `agents`

Fix:

- correct the YAML format
- remove unsupported keys
- move metadata to a supported resource kind

## `agents/openai.yaml` authored directly inside a skill

Cause:

- package skills must not include a hand-written `agents/openai.yaml`

Fix:

- remove the authored sidecar
- express Codex skill sidecar data through supported `codex.interface`,
  `codex.policy`, and `codex.dependencies` frontmatter fields instead

## Drift overwrite prompts during `ply apply`

Cause:

- an exposed file already managed by Ply differs from the current desired
  output

Fix:

- inspect `ply apply --dry-run` and `ply diff`
- accept the overwrite if the package is the new source of truth
- keep the file unchanged if the local drift is intentional

Use `ply apply --yes` only when you want to accept all overwrite prompts.

## `ply doctor` warns about ignore coverage

Cause:

- the managed Ply block in `.git/info/exclude` is missing or incomplete

Fix:

- re-run `ply init`
- or re-run `ply apply`, which also refreshes local exclude coverage based on
  the current state settings

## Git source issues

Common causes:

- `repo` is malformed
- a local repo path cannot be resolved
- a local repo path was given with a `rev` other than `HEAD`
- SSH transport settings do not match the selected remote form

Fix:

- verify the `repo` value
- use `HEAD` or omit `rev` for local repo paths
- verify `ply.ssh.toml` if you expect GitHub shorthand to resolve over SSH

## `ply apply` did not advance a Git source

Cause:

- `ply.lock` already contains a matching lock entry for that Git source
- `ply apply` is lock-aware and reuses the locked revision

Fix:

- run `ply update`
- or run `ply update <source-id>` to advance one locked Git source
- then run `ply apply` to expose assets from the new locked revision

## `ply update <source-id>` refuses to preserve other Git sources

Cause:

- `ply.lock` is missing a previous revision for another configured Git source
- or the current source locator no longer matches the locator recorded in
  `ply.lock`

Fix:

- run `ply update` without a source id to rewrite the full lockfile with the
  current source locators and resolved revisions

## Related docs

- [Consume packages in a project repo](../guides/consume-packages-project.md)
- [Create your first package](../guides/create-package.md)
- [CLI command reference](cli.md)
