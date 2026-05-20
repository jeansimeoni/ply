# Package Format And Asset Kinds

This reference describes the on-disk format of a Ply package.

## Package root

Each package root must contain:

```txt
ply-package.toml
```

Minimal example:

```toml
name = "review-tools"
```

Supported manifest fields today:

- `name`
- `version`
- `description`

Each configured source points at one package root. A source is invalid if
`ply-package.toml` is missing from the source root.

## Supported top-level asset kinds

Ply currently recognizes these package-level asset kinds:

- `commands/`
- `skills/`
- `agents/`
- `rules/`
- `hooks/`
- `output-styles/`
- `local-instructions.md`

Directory-based kinds:

- `commands`
- `skills`
- `agents`
- `rules`
- `hooks`
- `output-styles`

File-based kind:

- `local-instructions`

## Required filenames by kind

Some kinds use a required primary Markdown filename inside each resource:

- `skills/<name>/SKILL.md`
- `agents/<name>/AGENT.md`

`commands/` and `output-styles/` use Markdown files directly in their kind
directory.

Example:

```txt
review-tools/
├── ply-package.toml
├── commands/
│   └── pr-review.md
├── agents/
│   └── reviewer/
│       └── AGENT.md
└── skills/
    └── review-diff/
        └── SKILL.md
```

## Prompt-resource kinds

These kinds share Ply's YAML frontmatter authoring model:

- `commands`
- `skills`
- `agents`
- `output-styles`

`rules` and `hooks` are native adapter resources and do not use this shared
frontmatter model.

## Shared frontmatter keys

Supported shared keys:

- `name`
- `description`
- `category`
- `argument-hint` or `argument_hint`
- `keep-coding-instructions` or `keep_coding_instructions`

Restrictions:

- `argument-hint` is only valid for `commands`.
- `keep-coding-instructions` is only valid for `output-styles`.

Example:

```md
---
name: technical-writer
description: Write clear technical documentation
---

Write accurate documentation with verifiable examples.
```

## Claude frontmatter keys

Supported `claude:` keys:

- `model`
- `tools`
- `argument-hint` or `argument_hint`
- `disable-model-invocation` or `disable_model_invocation`
- `context`
- `agent`
- `user-invocable` or `user_invocable`
- `disallowedTools` or `disallowed_tools`
- `mcpServers` or `mcp_servers`

Restrictions:

- `context`, `agent`, `disable-model-invocation`, and `user-invocable` are only
  valid for `commands` and `skills`.
- `disallowedTools` and `mcpServers` are only valid for `agents`.

## Codex frontmatter keys

Supported `codex:` keys:

- `model`
- `reasoning_effort` or `reasoning-effort`
- `tools`
- `sandbox_mode` or `sandbox-mode`
- `approval_policy` or `approval-policy`
- `mcp_servers` or `mcp-servers`
- `interface`
- `policy`
- `dependencies`

Restrictions:

- `sandbox_mode`, `approval_policy`, and `mcp_servers` are only valid for
  `agents`.
- `interface`, `policy`, and `dependencies` are only valid for `skills`.

Unknown frontmatter keys are rejected.

## Per-resource adapter targeting

Resources target all enabled adapters unless you add metadata that limits them.

Use:

- `skills/<name>/ply-asset.toml` for directory resources
- `<file>.ply-asset.toml` for file resources

Example:

```toml
targets = ["codex"]
```

If `targets` is omitted or empty, the resource applies to all enabled adapters.

## Naming and exposed prefixes

Package authors use natural names in the package root. When Ply writes into
adapter-owned namespaces, it adds the `ply-` prefix where required.

Examples:

- `skills/review-diff/` becomes `.claude/skills/ply-review-diff/`
- `agents/reviewer/` becomes `.claude/agents/ply-reviewer/`
- `commands/pr-review.md` becomes `.agents/commands/ply-pr-review.md`

## Adapter outputs

Current adapter mapping:

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
- `agents` -> `.codex/agents/*.toml`
- `local-instructions` -> `AGENTS.override.md`
- `rules` -> `.codex/rules/`
- `hooks` -> `.codex/hooks/` plus `.codex/hooks.json`
- `output-styles` -> `AGENTS.override.md`

## Generated adapter-specific behavior

For shared prompt resources, Ply renders adapter-specific output from the
authored Markdown:

- Claude receives adapter-native Markdown where applicable.
- Codex skills receive generated `SKILL.md` frontmatter and may also receive
  generated `agents/openai.yaml` when `codex.interface`, `codex.policy`, or
  `codex.dependencies` are present.
- Codex agents receive generated `.codex/agents/<name>.toml`.
- Codex commands and Codex output styles can receive a generated Markdown
  settings preamble based on Codex metadata.

Package authors must not author `agents/openai.yaml` directly inside a skill.

## Related docs

- [Create your first package](../guides/create-package.md)
- [Configuration and layering](configuration-and-layering.md)
