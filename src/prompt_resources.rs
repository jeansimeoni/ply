use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::path::Path;

use crate::adapters::AssetKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPromptResource {
    pub logical_name: String,
    pub body: String,
    pub shared: SharedPromptMetadata,
    pub claude: ClaudePromptMetadata,
    pub codex: CodexPromptMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedPromptMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub argument_hint: Option<String>,
    pub keep_coding_instructions: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudePromptMetadata {
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub argument_hint: Option<String>,
    pub disable_model_invocation: Option<bool>,
    pub context: Option<String>,
    pub agent: Option<String>,
    pub user_invocable: Option<bool>,
    pub disallowed_tools: Vec<String>,
    pub mcp_servers: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexPromptMetadata {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub tools: Vec<String>,
    pub sandbox_mode: Option<String>,
    pub approval_policy: Option<String>,
    pub mcp_servers: Option<Value>,
    pub interface: Option<Value>,
    pub policy: Option<Value>,
    pub dependencies: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodexAgentFile {
    pub name: String,
    pub description: String,
    pub developer_instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<toml::Value>,
}

pub fn is_prompt_resource(kind: AssetKind) -> bool {
    matches!(
        kind,
        AssetKind::Commands | AssetKind::Skills | AssetKind::Agents | AssetKind::OutputStyles
    )
}

pub fn primary_markdown_name(kind: AssetKind) -> Option<&'static str> {
    match kind {
        AssetKind::Skills => Some("SKILL.md"),
        AssetKind::Agents => Some("AGENT.md"),
        _ => None,
    }
}

pub fn parse_prompt_resource(
    kind: AssetKind,
    logical_name: &str,
    markdown: &str,
) -> Result<ParsedPromptResource> {
    let (frontmatter, body) = parse_frontmatter(markdown)?;
    let mut shared = SharedPromptMetadata::default();
    let mut claude = ClaudePromptMetadata::default();
    let mut codex = CodexPromptMetadata::default();

    if let Some(mut map) = frontmatter {
        shared.name = take_string(&mut map, &["name"])?;
        shared.description = take_string(&mut map, &["description"])?;
        shared.category = take_string(&mut map, &["category"])?;
        shared.argument_hint =
            take_inline_string(&mut map, &["argument-hint", "argument_hint"])?;
        shared.keep_coding_instructions = take_bool(
            &mut map,
            &["keep-coding-instructions", "keep_coding_instructions"],
        )?;

        if let Some(mut section) = take_mapping(&mut map, &["claude"])? {
            claude.model = take_string(&mut section, &["model"])?;
            claude.tools = take_string_list(&mut section, &["tools"])?;
            claude.argument_hint =
                take_inline_string(&mut section, &["argument-hint", "argument_hint"])?;
            claude.disable_model_invocation = take_bool(
                &mut section,
                &[
                    "disable-model-invocation",
                    "disable_model_invocation",
                ],
            )?;
            claude.context = take_string(&mut section, &["context"])?;
            claude.agent = take_string(&mut section, &["agent"])?;
            claude.user_invocable =
                take_bool(&mut section, &["user-invocable", "user_invocable"])?;
            claude.disallowed_tools =
                take_string_list(&mut section, &["disallowedTools", "disallowed_tools"])?;
            claude.mcp_servers = take_value(&mut section, &["mcpServers", "mcp_servers"])?;
            reject_unknown_keys("claude", &section)?;
        }

        if let Some(mut section) = take_mapping(&mut map, &["codex"])? {
            codex.model = take_string(&mut section, &["model"])?;
            codex.reasoning_effort =
                take_string(&mut section, &["reasoning_effort", "reasoning-effort"])?;
            codex.tools = take_string_list(&mut section, &["tools"])?;
            codex.sandbox_mode = take_string(&mut section, &["sandbox_mode", "sandbox-mode"])?;
            codex.approval_policy =
                take_string(&mut section, &["approval_policy", "approval-policy"])?;
            codex.mcp_servers = take_value(&mut section, &["mcp_servers", "mcp-servers"])?;
            codex.interface = take_value(&mut section, &["interface"])?;
            codex.policy = take_value(&mut section, &["policy"])?;
            codex.dependencies = take_value(&mut section, &["dependencies"])?;
            reject_unknown_keys("codex", &section)?;
        }

        reject_unknown_keys("frontmatter", &map)?;
    }

    validate_metadata(kind, &shared, &claude, &codex)?;

    Ok(ParsedPromptResource {
        logical_name: logical_name.to_string(),
        body: body.trim().to_string(),
        shared,
        claude,
        codex,
    })
}

pub fn render_claude_markdown(kind: AssetKind, resource: &ParsedPromptResource) -> Result<String> {
    let mut map = Mapping::new();
    if let Some(name) = &resource.shared.name {
        map.insert(Value::String("name".to_string()), Value::String(name.clone()));
    }
    if let Some(description) = &resource.shared.description {
        map.insert(
            Value::String("description".to_string()),
            Value::String(description.clone()),
        );
    }

    match kind {
        AssetKind::Commands | AssetKind::Skills => {
            let argument_hint = resource
                .claude
                .argument_hint
                .as_ref()
                .or(resource.shared.argument_hint.as_ref());
            if let Some(argument_hint) = argument_hint {
                map.insert(
                    Value::String("argument-hint".to_string()),
                    Value::String(argument_hint.clone()),
                );
            }
            if let Some(disable) = resource.claude.disable_model_invocation {
                map.insert(
                    Value::String("disable-model-invocation".to_string()),
                    Value::Bool(disable),
                );
            }
            if let Some(context) = &resource.claude.context {
                map.insert(
                    Value::String("context".to_string()),
                    Value::String(context.clone()),
                );
            }
            if let Some(agent) = &resource.claude.agent {
                map.insert(
                    Value::String("agent".to_string()),
                    Value::String(agent.clone()),
                );
            }
            if let Some(user_invocable) = resource.claude.user_invocable {
                map.insert(
                    Value::String("user-invocable".to_string()),
                    Value::Bool(user_invocable),
                );
            }
            if !resource.claude.tools.is_empty() {
                map.insert(
                    Value::String("allowed-tools".to_string()),
                    yaml_sequence(&resource.claude.tools),
                );
            }
            if let Some(model) = &resource.claude.model {
                map.insert(
                    Value::String("model".to_string()),
                    Value::String(model.clone()),
                );
            }
        }
        AssetKind::Agents => {
            if let Some(model) = &resource.claude.model {
                map.insert(
                    Value::String("model".to_string()),
                    Value::String(model.clone()),
                );
            }
            if !resource.claude.tools.is_empty() {
                map.insert(
                    Value::String("tools".to_string()),
                    yaml_sequence(&resource.claude.tools),
                );
            }
            if !resource.claude.disallowed_tools.is_empty() {
                map.insert(
                    Value::String("disallowedTools".to_string()),
                    yaml_sequence(&resource.claude.disallowed_tools),
                );
            }
            if let Some(mcp_servers) = &resource.claude.mcp_servers {
                map.insert(
                    Value::String("mcpServers".to_string()),
                    mcp_servers.clone(),
                );
            }
        }
        AssetKind::OutputStyles => {
            if let Some(keep) = resource.shared.keep_coding_instructions {
                map.insert(
                    Value::String("keep-coding-instructions".to_string()),
                    Value::Bool(keep),
                );
            }
            if let Some(model) = &resource.claude.model {
                map.insert(
                    Value::String("model".to_string()),
                    Value::String(model.clone()),
                );
            }
            if !resource.claude.tools.is_empty() {
                map.insert(
                    Value::String("tools".to_string()),
                    yaml_sequence(&resource.claude.tools),
                );
            }
        }
        _ => return Err(anyhow!("unsupported prompt resource kind `{}`", kind.as_str())),
    }

    render_markdown_document(map, &resource.body)
}

pub fn render_codex_agent(resource: &ParsedPromptResource) -> Result<String> {
    let agent = CodexAgentFile {
        name: resource
            .shared
            .name
            .clone()
            .unwrap_or_else(|| resource.logical_name.clone()),
        description: resource
            .shared
            .description
            .clone()
            .unwrap_or_else(|| infer_description(&resource.logical_name, &resource.body)),
        developer_instructions: resource.body.clone(),
        model: resource.codex.model.clone(),
        model_reasoning_effort: resource.codex.reasoning_effort.clone(),
        sandbox_mode: resource.codex.sandbox_mode.clone(),
        approval_policy: resource.codex.approval_policy.clone(),
        mcp_servers: resource
            .codex
            .mcp_servers
            .as_ref()
            .map(yaml_to_toml)
            .transpose()?,
    };
    toml::to_string_pretty(&agent).map_err(Into::into)
}

pub fn render_codex_skill_sidecar(resource: &ParsedPromptResource) -> Result<Option<String>> {
    if resource.codex.interface.is_none()
        && resource.codex.policy.is_none()
        && resource.codex.dependencies.is_none()
    {
        return Ok(None);
    }
    let mut map = Mapping::new();
    if let Some(interface) = &resource.codex.interface {
        map.insert(Value::String("interface".to_string()), interface.clone());
    }
    if let Some(policy) = &resource.codex.policy {
        map.insert(Value::String("policy".to_string()), policy.clone());
    }
    if let Some(dependencies) = &resource.codex.dependencies {
        map.insert(
            Value::String("dependencies".to_string()),
            dependencies.clone(),
        );
    }
    Ok(Some(serde_yaml::to_string(&Value::Mapping(map))?))
}

pub fn render_codex_prompt_preamble(resource: &ParsedPromptResource) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(model) = &resource.codex.model {
        lines.push(format!("Preferred model: {model}"));
    }
    if let Some(reasoning_effort) = &resource.codex.reasoning_effort {
        lines.push(format!("Reasoning effort: {reasoning_effort}"));
    }
    if !resource.codex.tools.is_empty() {
        lines.push(format!(
            "Preferred tools: {}",
            resource.codex.tools.join(", ")
        ));
    }
    if let Some(sandbox_mode) = &resource.codex.sandbox_mode {
        lines.push(format!("Sandbox mode: {sandbox_mode}"));
    }
    if let Some(approval_policy) = &resource.codex.approval_policy {
        lines.push(format!("Approval policy: {approval_policy}"));
    }

    if lines.is_empty() {
        return None;
    }

    let mut rendered = String::from("## Ply Codex Settings\n\n");
    rendered.push_str(&lines.join("\n"));
    rendered.push_str("\n\n---\n\n");
    Some(rendered)
}

fn validate_metadata(
    kind: AssetKind,
    shared: &SharedPromptMetadata,
    claude: &ClaudePromptMetadata,
    codex: &CodexPromptMetadata,
) -> Result<()> {
    if shared.argument_hint.is_some() && !matches!(kind, AssetKind::Commands) {
        return Err(anyhow!(
            "`argument-hint` is only supported for `commands` resources"
        ));
    }
    if shared.keep_coding_instructions.is_some() && !matches!(kind, AssetKind::OutputStyles) {
        return Err(anyhow!(
            "`keep-coding-instructions` is only supported for `output-styles` resources"
        ));
    }
    if (claude.context.is_some()
        || claude.agent.is_some()
        || claude.disable_model_invocation.is_some()
        || claude.user_invocable.is_some())
        && !matches!(kind, AssetKind::Commands | AssetKind::Skills)
    {
        return Err(anyhow!(
            "Claude command/skill fields are only supported for `commands` and `skills` resources"
        ));
    }
    if (!claude.disallowed_tools.is_empty() || claude.mcp_servers.is_some())
        && !matches!(kind, AssetKind::Agents)
    {
        return Err(anyhow!(
            "Claude agent-only fields are only supported for `agents` resources"
        ));
    }
    if (codex.interface.is_some() || codex.policy.is_some() || codex.dependencies.is_some())
        && !matches!(kind, AssetKind::Skills)
    {
        return Err(anyhow!(
            "Codex skill sidecar fields are only supported for `skills` resources"
        ));
    }
    if (codex.sandbox_mode.is_some()
        || codex.approval_policy.is_some()
        || codex.mcp_servers.is_some())
        && !matches!(kind, AssetKind::Agents)
    {
        return Err(anyhow!(
            "Codex agent runtime fields are only supported for `agents` resources"
        ));
    }
    Ok(())
}

fn parse_frontmatter(markdown: &str) -> Result<(Option<Mapping>, String)> {
    if !markdown.starts_with("---\n") {
        return Ok((None, markdown.to_string()));
    }
    let rest = &markdown[4..];
    let Some(end) = rest.find("\n---\n") else {
        return Err(anyhow!("unterminated YAML frontmatter"));
    };
    let frontmatter = &rest[..end];
    let body = rest[end + 5..].to_string();
    let value: Value = serde_yaml::from_str(frontmatter)?;
    let Value::Mapping(map) = value else {
        return Err(anyhow!("frontmatter must be a YAML mapping"));
    };
    Ok((Some(map), body))
}

fn render_markdown_document(map: Mapping, body: &str) -> Result<String> {
    if map.is_empty() {
        return Ok(if body.is_empty() {
            String::new()
        } else {
            format!("{}\n", body.trim_end())
        });
    }
    let frontmatter = serde_yaml::to_string(&Value::Mapping(map))?;
    Ok(format!("---\n{}---\n\n{}\n", frontmatter, body.trim()))
}

fn yaml_sequence(values: &[String]) -> Value {
    Value::Sequence(values.iter().map(|value| Value::String(value.clone())).collect())
}

fn infer_description(logical_name: &str, body: &str) -> String {
    let mut lines = body.lines().peekable();
    while let Some(line) = lines.peek() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.next();
            continue;
        }
        let mut description = String::new();
        while let Some(line) = lines.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(trimmed);
            lines.next();
        }
        if !description.is_empty() {
            return description;
        }
    }
    format!("Ply-managed resource `{logical_name}`.")
}

fn take_string(map: &mut Mapping, keys: &[&str]) -> Result<Option<String>> {
    let Some(value) = remove_key(map, keys) else {
        return Ok(None);
    };
    match value {
        Value::String(value) => Ok(Some(value)),
        other => Err(anyhow!("expected string for `{}`; got {:?}", keys[0], other)),
    }
}

fn take_inline_string(map: &mut Mapping, keys: &[&str]) -> Result<Option<String>> {
    let Some(value) = remove_key(map, keys) else {
        return Ok(None);
    };
    match value {
        Value::String(value) => Ok(Some(value)),
        Value::Sequence(values) => Ok(Some(
            values
                .into_iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value),
                    other => Err(anyhow!(
                        "expected string entries for `{}`; got {:?}",
                        keys[0],
                        other
                    )),
                })
                .collect::<Result<Vec<_>>>()?
                .join(" "),
        )),
        other => Err(anyhow!("expected string for `{}`; got {:?}", keys[0], other)),
    }
}

fn take_bool(map: &mut Mapping, keys: &[&str]) -> Result<Option<bool>> {
    let Some(value) = remove_key(map, keys) else {
        return Ok(None);
    };
    match value {
        Value::Bool(value) => Ok(Some(value)),
        other => Err(anyhow!("expected boolean for `{}`; got {:?}", keys[0], other)),
    }
}

fn take_string_list(map: &mut Mapping, keys: &[&str]) -> Result<Vec<String>> {
    let Some(value) = remove_key(map, keys) else {
        return Ok(Vec::new());
    };
    match value {
        Value::String(value) => Ok(value
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect()),
        Value::Sequence(values) => values
            .into_iter()
            .map(|value| match value {
                Value::String(value) => Ok(value),
                other => Err(anyhow!(
                    "expected string entries for `{}`; got {:?}",
                    keys[0],
                    other
                )),
            })
            .collect(),
        other => Err(anyhow!(
            "expected string or sequence for `{}`; got {:?}",
            keys[0],
            other
        )),
    }
}

fn take_mapping(map: &mut Mapping, keys: &[&str]) -> Result<Option<Mapping>> {
    let Some(value) = remove_key(map, keys) else {
        return Ok(None);
    };
    match value {
        Value::Mapping(value) => Ok(Some(value)),
        other => Err(anyhow!("expected mapping for `{}`; got {:?}", keys[0], other)),
    }
}

fn take_value(map: &mut Mapping, keys: &[&str]) -> Result<Option<Value>> {
    Ok(remove_key(map, keys))
}

fn remove_key(map: &mut Mapping, keys: &[&str]) -> Option<Value> {
    for key in keys {
        let key_value = Value::String((*key).to_string());
        if let Some(value) = map.remove(&key_value) {
            return Some(value);
        }
    }
    None
}

fn reject_unknown_keys(scope: &str, map: &Mapping) -> Result<()> {
    if map.is_empty() {
        return Ok(());
    }
    let keys = map
        .keys()
        .map(|key| match key {
            Value::String(value) => value.clone(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!("unsupported keys in {scope}: {keys}"))
}

fn yaml_to_toml(value: &Value) -> Result<toml::Value> {
    match value {
        Value::Null => Err(anyhow!("null is not supported in Codex TOML metadata")),
        Value::Bool(value) => Ok(toml::Value::Boolean(*value)),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Ok(toml::Value::Integer(integer))
            } else if let Some(float) = value.as_f64() {
                Ok(toml::Value::Float(float))
            } else {
                Err(anyhow!("unsupported numeric value in YAML metadata"))
            }
        }
        Value::String(value) => Ok(toml::Value::String(value.clone())),
        Value::Sequence(values) => Ok(toml::Value::Array(
            values
                .iter()
                .map(yaml_to_toml)
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Mapping(map) => {
            let mut table = toml::map::Map::new();
            for (key, value) in map {
                let Value::String(key) = key else {
                    return Err(anyhow!("YAML metadata keys must be strings"));
                };
                table.insert(key.clone(), yaml_to_toml(value)?);
            }
            Ok(toml::Value::Table(table))
        }
        Value::Tagged(tagged) => yaml_to_toml(&tagged.value),
    }
}

pub fn prompt_logical_name(path: &Path) -> Result<String> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid prompt resource path `{}`", path.display()))?;
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_normalizes_tool_lists_and_sections() -> Result<()> {
        let resource = parse_prompt_resource(
            AssetKind::Agents,
            "ply-reviewer",
            r#"---
name: reviewer
description: Review code
claude:
  tools: Read, Write
codex:
  tools:
    - shell
    - patch
  sandbox_mode: workspace-write
---

Review carefully.
"#,
        )?;

        assert_eq!(resource.shared.name.as_deref(), Some("reviewer"));
        assert_eq!(resource.claude.tools, vec!["Read", "Write"]);
        assert_eq!(resource.codex.tools, vec!["shell", "patch"]);
        assert_eq!(
            resource.codex.sandbox_mode.as_deref(),
            Some("workspace-write")
        );
        Ok(())
    }

    #[test]
    fn reject_unknown_frontmatter_keys() {
        let err = parse_prompt_resource(
            AssetKind::Commands,
            "ply-docs",
            "---\nunknown: true\n---\n\nBody\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unsupported keys in frontmatter"));
    }

    #[test]
    fn reject_wrong_kind_codex_skill_fields() {
        let err = parse_prompt_resource(
            AssetKind::Commands,
            "ply-docs",
            r#"---
codex:
  interface:
    display_name: Docs
---

Body
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Codex skill sidecar fields"));
    }
}
