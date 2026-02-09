use crate::engine::TaskPacket;
use anyhow::Result;
use serde::Deserialize;
use serde_json::de::Deserializer;

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
    #[serde(rename = "ask")]
    Ask { question: String },
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
    for (idx, _) in trimmed.match_indices('{') {
        let candidate = &trimmed[idx..];
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
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
        return Some(ModelAction::Tool {
            tool: tool.to_string(),
            args,
        });
    }

    let tool_name = match action_type {
        "read_file"
        | "write_file"
        | "list_files"
        | "search_rg"
        | "run_command"
        | "capture_screenshot"
        | "acquire_locks"
        | "unlock_paths"
        | "delegate_to_agent"
        | "get_repo_info" => action_type,
        _ => return None,
    };

    Some(ModelAction::Tool {
        tool: tool_name.to_string(),
        args,
    })
}
