use crate::engine::TaskPacket;
use anyhow::Result;
use serde::Deserialize;
use serde_json::de::Deserializer;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ModelAction {
    #[serde(rename = "tool")]
    Tool {
        tool: String,
        args: serde_json::Value,
    },
    #[serde(rename = "patch")]
    Patch { diff: String },
    #[serde(rename = "finalize_task")]
    FinalizeTask { packet: TaskPacket },
}

pub fn parse_first_action(raw: &str) -> Result<ModelAction> {
    let trimmed = raw.trim();

    // Fast path: a single clean JSON object.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(action) = value_to_action(value) {
            return Ok(action);
        }
    }

    // Fallback: models sometimes emit prose + one or more JSON objects.
    // Scan for JSON object starts and return the first valid ModelAction.
    // We strip XML-style tags like <search_indexing> before parsing.
    let mut cleaned = trimmed.to_string();
    while let Some(start) = cleaned.find('<') {
        if let Some(end) = cleaned[start..].find('>') {
            cleaned.replace_range(start..start + end + 1, "");
        } else {
            break;
        }
    }
    let cleaned_trimmed = cleaned.trim();

    for (idx, _) in cleaned_trimmed.match_indices('{') {
        let candidate = &cleaned_trimmed[idx..];
        let stream = Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        for value in stream.flatten() {
            if let Some(action) = value_to_action(value) {
                return Ok(action);
            }
        }
    }

    anyhow::bail!("no valid model action found in response")
}

fn value_to_action(value: serde_json::Value) -> Option<ModelAction> {
    if let Ok(action) = serde_json::from_value::<ModelAction>(value.clone()) {
        return Some(action);
    }

    let obj = value.as_object()?;
    let action_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .or_else(|| obj.get("tool_args"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
        if !is_supported_model_tool(tool) {
            return None;
        }
        return Some(ModelAction::Tool {
            tool: tool.to_string(),
            args,
        });
    }

    let tool_name = match action_type {
        "Read" | "Grep" | "Write" | "Glob" | "Bash" | "capture_screenshot" | "lock_paths"
        | "unlock_paths" | "delegate_to_agent" | "get_repo_info" => action_type,
        _ => return None,
    };

    Some(ModelAction::Tool {
        tool: tool_name.to_string(),
        args,
    })
}

fn is_supported_model_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Read"
            | "Grep"
            | "Write"
            | "Glob"
            | "Bash"
            | "capture_screenshot"
            | "lock_paths"
            | "unlock_paths"
            | "delegate_to_agent"
            | "get_repo_info"
    )
}

fn strip_angle_tags(input: &str) -> String {
    // Keep behavior aligned with parse_first_action's tag stripping.
    let mut cleaned = input.to_string();
    while let Some(start) = cleaned.find('<') {
        if let Some(end) = cleaned[start..].find('>') {
            cleaned.replace_range(start..start + end + 1, "");
        } else {
            break;
        }
    }
    cleaned
}

fn extract_first_json_object_span(s: &str) -> Option<(usize, usize)> {
    // Return byte offsets [start,end) for the first JSON object.
    // This is a best-effort extractor for logging. It handles nested braces and quoted strings.
    let bytes = s.as_bytes();
    let mut start: Option<usize> = None;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        if start.is_none() {
            if c == '{' {
                start = Some(i);
                depth = 1;
            }
            continue;
        }

        if escape {
            escape = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start.unwrap(), i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn truncate_text_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    format!("{prefix}…")
}

fn truncate_json_values(value: Value, max_chars: usize) -> Value {
    match value {
        Value::String(s) => Value::String(truncate_text_chars(&s, max_chars)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| truncate_json_values(v, max_chars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, truncate_json_values(v, max_chars)))
                .collect(),
        ),
        other => other,
    }
}

/// For logging/debugging only: split a model message into (text,json) parts with truncation.
///
/// - **text**: the non-JSON portion, truncated to `max_text_chars`
/// - **json**: the first JSON object (if any), with each string value truncated to `max_json_value_chars`
pub fn model_message_log_parts(
    raw: &str,
    max_text_chars: usize,
    max_json_value_chars: usize,
) -> (String, Option<Value>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("".to_string(), None);
    }

    let cleaned = strip_angle_tags(trimmed);
    let cleaned_trimmed = cleaned.trim();
    let Some((start, end)) = extract_first_json_object_span(cleaned_trimmed) else {
        return (truncate_text_chars(cleaned_trimmed, max_text_chars), None);
    };

    let json_str = &cleaned_trimmed[start..end];
    let before = cleaned_trimmed[..start].trim();
    let after = cleaned_trimmed[end..].trim();
    let text = [before, after]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let json_value = serde_json::from_str::<Value>(json_str)
        .ok()
        .map(|v| truncate_json_values(v, max_json_value_chars));
    (truncate_text_chars(&text, max_text_chars), json_value)
}
