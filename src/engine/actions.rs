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
    // IMPORTANT: do not strip angle-bracket content here.
    // Rust/TS code in tool args can contain generics like `Option<u64>` and
    // removing `<...>` would corrupt write payloads.
    let cleaned_trimmed = trimmed;

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
        "Read" | "Grep" | "Write" | "Edit" | "Glob" | "Bash" | "capture_screenshot"
        | "lock_paths" | "unlock_paths" | "delegate_to_agent" | "get_repo_info" => action_type,
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
            | "Edit"
            | "Glob"
            | "Bash"
            | "capture_screenshot"
            | "lock_paths"
            | "unlock_paths"
            | "delegate_to_agent"
            | "get_repo_info"
    )
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

    let cleaned_trimmed = trimmed;
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

#[cfg(test)]
mod tests {
    use super::{parse_first_action, ModelAction};

    #[test]
    fn parse_first_action_preserves_generic_types_in_write_content() {
        let raw = r#"I'll apply the fix.
{"type":"tool","tool":"Write","args":{"path":"src/logging.rs","content":"static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();\npub struct LoggingSettings<'a> { pub level: Option<&'a str>, pub retention_days: Option<u64>, }\n"}} "#;

        let action = parse_first_action(raw).expect("expected tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Write");
                let content = args["content"].as_str().expect("content should be a string");
                assert!(content.contains("OnceLock<WorkerGuard>"));
                assert!(content.contains("Option<&'a str>"));
                assert!(content.contains("Option<u64>"));
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_first_action_handles_wrapped_json_without_stripping() {
        let raw =
            "<search_indexing>\n{\"type\":\"tool\",\"tool\":\"Read\",\"args\":{\"path\":\"src/logging.rs\"}}\n</search_indexing>";
        let action = parse_first_action(raw).expect("expected wrapped tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Read");
                assert_eq!(args["path"], "src/logging.rs");
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_first_action_accepts_edit_tool() {
        let raw = r#"{"type":"tool","tool":"Edit","args":{"path":"src/logging.rs","old_string":"foo","new_string":"bar","replace_all":false}}"#;
        let action = parse_first_action(raw).expect("expected Edit tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Edit");
                assert_eq!(args["path"], "src/logging.rs");
                assert_eq!(args["old_string"], "foo");
                assert_eq!(args["new_string"], "bar");
            }
            _ => panic!("expected tool action"),
        }
    }
}
