# Inspect, Update, And Clean Managed Assets

This guide covers the ongoing maintenance commands you use after a repo or the
global root is already initialized.

## Preview changes before writing

Use:

```bash
ply apply --dry-run
```

This is the safest way to inspect the current composition. The dry-run reports:

- resolved sources
- resolved packages
- overlays that apply
- the managed assets Ply plans to expose
- managed content drift that would require consent

Use it when:

- a source revision changed
- you edited `ply.toml`, `ply.local.toml`, or `ply.ssh.toml`
- you changed package contents
- you want to understand what a real `apply` would do

## Apply updates

Use:

```bash
ply apply
```

Ply writes generated files, updates exposed managed files, refreshes
`ply.lock`, and updates `.ply/state.json`.

For Git sources, `apply` reuses the locked revision already recorded in
`ply.lock` when the source locator still matches. It does not
opportunistically advance locked Git sources.

If a file is already managed by Ply but its exposed contents differ from the
new desired result, Ply treats that as drift and asks before overwriting that
file.

Use:

```bash
ply update
ply update team
ply update -g
```

when you want to advance locked Git revisions in `ply.lock` without writing
managed assets.

After `ply update`, run `ply apply` to expose assets from the new locked
revisions.

Use:

```bash
ply apply --yes
```

only when you want to accept all overwrite prompts for drifted managed files.

## Read the drift report

Use:

```bash
ply diff
```

`ply diff` compares the current repo state to the current desired state based
on the resolved packages and overlays.

Use it when:

- you want to see whether managed files still match source packages
- you need a quick check before committing or sharing a repo state
- you suspect that exposed files were edited manually

If it reports `no differences`, the managed outputs match the current desired
composition.

## Run health checks

Use:

```bash
ply doctor
```

For a project root, `doctor` validates:

- the manifest schema and adapter list
- source resolution
- package root validity
- ignore coverage for local-only Ply files
- state drift for owned files

Use:

```bash
ply doctor -g
```

to validate the user-global root instead.

## Inspect package and source resolution

Use:

```bash
ply list
```

to see which package roots resolved.

Use:

```bash
ply sources
```

to see which sources resolved and, for Git sources, which pinned revisions were
recorded.

These commands are useful when:

- a source override in `ply.local.toml` is not taking effect
- a project and global layer both define sources
- you want to confirm the exact source locator and revision used by the last
  composition

## Clean managed files safely

Preview removals:

```bash
ply clean --dry-run
```

Remove Ply-managed files:

```bash
ply clean
```

`ply nuke` is an alias for `ply clean`.

Cleanup removes Ply-managed files and local state, including:

- generated and cached data under `.ply/`
- `ply.toml`
- `ply.lock`
- the scaffolded `ply-packages/example-review` fixture if it exists
- Ply-managed assets in `.claude/`, `.codex/`, and `.agents/`
- the Ply-managed block in `.git/info/exclude`

Cleanup does not remove repository-owned files such as:

- `AGENTS.md`
- `CLAUDE.md`
- unrelated files in adapter-owned directories

Because cleanup is destructive, Ply asks for confirmation unless you provide
`--yes`.

## Common workflow

For routine maintenance in a repo:

```bash
ply apply --dry-run
ply diff
ply doctor
ply apply
```

Use `clean --dry-run` before removing Ply from a repo or global root.

## Next steps

- For project setup, see
  [Consume packages in a project repo](consume-packages-project.md).
- For command syntax, see [CLI command reference](../reference/cli.md).
- For error lookup, see [Troubleshooting](../reference/troubleshooting.md).
