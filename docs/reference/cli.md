# CLI Command Reference

This page summarizes the current MVP CLI surface.

## Notes

- Project-mode commands expect `ply.toml` to exist unless the command is
  `ply init`.
- Project-mode `init`, `apply`, and `clean` require the current directory to be
  a Git repository.
- The docs assume the `ply` binary is already installed and available on your
  `PATH`.

## `ply init`

Initialize Ply in the current project repo or the user-global root.

```txt
ply init [options]
ply init package [options]
```

Options:

- `--with-packages`
- `--without-packages`
- `--adapters <list>`
- `--ignore-config`
- `--track-config`
- `--global`, `-g`
- `--dry-run`
- `--yes`, `-y`

Use `ply init` for consumer setup. Use `ply init package` for authoring a
package root.

Notes:

- `--adapters` accepts a comma-separated subset of `codex,claude`
- if omitted, Ply initializes the manifest with both adapters enabled
- `ply init package` can create only `ply-package.toml`, but that package
  root will not pass package validation until you add at least one supported
  managed asset

## `ply init package`

Initialize one reusable package root.

```txt
ply init package [options]
```

Options:

- `--name <name>`
- `--path <dir>`
- `--kinds <list>`
- `--dry-run`

Notes:

- `--name` is required.
- `--kinds` is a comma-separated list such as `skills,commands,agents`.
- If `--kinds` is omitted, only `ply-package.toml` is created.
- `ply-package.toml` supports `name`, optional `version`, optional
  `description`, optional `license`, and optional `targets`.

## `ply apply`

Resolve sources and write managed assets.

```txt
ply apply [options]
```

Options:

- `--dry-run`
- `--yes`, `-y`

Notes:

- `--dry-run` previews layering, planned assets, and drift prompts.
- `--yes` accepts all overwrite prompts for drifted managed exposed files.
- For Git sources, `apply` reuses the locked revision from `ply.lock` when a
  matching lock entry is present.
- `apply` does not opportunistically advance locked Git revisions.
- A successful apply rewrites `ply.lock` with source locators plus resolved
  revisions and updates `.ply/state.json`.

## `ply add`

Add one source to `ply.toml`.

```txt
ply add [--global] --id <id> --path <path>
ply add [--global] --id <id> --git <repo> [--rev <rev>] [--ssh | --ssh-key <path>]
```

Notes:

- exactly one of `--path` or `--git` is required
- `--rev` is only valid with `--git`
- `--ssh` marks the Git source to use SSH transport with the default SSH key
- `--ssh-key <path>` marks the Git source to use SSH transport with a specific key path
- `--ssh` and `--ssh-key` update `ply.ssh.toml`
- adding a Git source also refreshes that source's `ply.lock` entry
- `--global`, `-g` mutates `~/.config/ply/ply.toml`

## `ply remove`

Remove one source from `ply.toml`.

```txt
ply remove [--global] <source-id> [--force]
```

Notes:

- `--force` is reserved for future compatibility
- successful removal prunes the matching `ply.lock` entry if present
- `--global`, `-g` mutates `~/.config/ply/ply.toml`

## `ply update`

Advance locked Git revisions and rewrite `ply.lock`.

```txt
ply update [--global] [source-id]
```

Notes:

- without a `source-id`, all configured Git sources are refreshed
- with a `source-id`, that source must resolve to a Git source
- `ply update` is the command that advances locked Git revisions
- `ply update` does not run `ply apply`
- `--global`, `-g` refreshes sources for `~/.config/ply`

## `ply diff`

Show differences between current managed outputs and the current desired state.

```txt
ply diff
```

Options:

- `--help`, `-h`

## `ply doctor`

Validate manifest, source resolution, package roots, and local safety checks.

```txt
ply doctor [options]
```

Options:

- `--global`, `-g`

Notes:

- package validation rejects unsupported adapter-owned directories such as
  `.claude/`, `.agents/`, and `.codex/` inside a package root
- package validation rejects package roots that expose no supported managed
  assets

## `ply list`

Show resolved package roots.

```txt
ply list [options]
```

Options:

- `--global`, `-g`

## `ply sources`

Show configured sources and their resolved locators and revisions.

```txt
ply sources [options]
```

Options:

- `--global`, `-g`

## `ply adapters`

Show the supported adapter mapping.

```txt
ply adapters
```

Options:

- `--help`, `-h`

## `ply clean`

Remove Ply-managed files from the project root or the user-global root.

```txt
ply clean [options]
ply nuke [options]
```

Options:

- `--global`, `-g`
- `--dry-run`
- `--yes`, `-y`

Notes:

- `ply nuke` is an alias for `ply clean`.
- `--dry-run` previews removals.
- without `--yes`, cleanup asks for destructive confirmation

## Related docs

- [Consume packages in a project repo](../guides/consume-packages-project.md)
- [Create your first package](../guides/create-package.md)
- [Troubleshooting](troubleshooting.md)
