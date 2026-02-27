use super::super::display::PlanDisplayItem;

/// Parse activity text from tool_status_line format into (tool_name, args_summary).
///
/// Maps patterns like:
///   "Reading file: src/main.rs" → ("Read", "src/main.rs")
///   "Running command: cargo test" → ("Bash", "cargo test")
///   "Searching: pattern" → ("Grep", "pattern")
pub(super) fn parse_activity_text(text: &str) -> (String, String) {
    let mappings: &[(&str, &str)] = &[
        ("Reading file: ", "Read"),
        ("Read file: ", "Read"),
        ("Read failed: ", "Read"),
        ("Writing file: ", "Write"),
        ("Wrote file: ", "Write"),
        ("Write failed: ", "Write"),
        ("Editing file: ", "Edit"),
        ("Edited file: ", "Edit"),
        ("Edit failed: ", "Edit"),
        ("Running command: ", "Bash"),
        ("Ran command: ", "Bash"),
        ("Command failed: ", "Bash"),
        ("Searching: ", "Grep"),
        ("Searched: ", "Grep"),
        ("Search failed: ", "Grep"),
        ("Listing files: ", "Glob"),
        ("Listed files: ", "Glob"),
        ("List files failed: ", "Glob"),
        ("Delegating to subagent: ", "Delegate"),
        ("Delegated to subagent: ", "Delegate"),
        ("Delegation failed: ", "Delegate"),
        ("Fetching URL: ", "WebFetch"),
        ("Fetched URL: ", "WebFetch"),
        ("Fetch failed: ", "WebFetch"),
        ("Searching web: ", "WebSearch"),
        ("Searched web: ", "WebSearch"),
        ("Web search failed: ", "WebSearch"),
        ("Calling tool: ", "Tool"),
        ("Used tool: ", "Tool"),
        ("Tool failed: ", "Tool"),
    ];

    for (prefix, tool_name) in mappings {
        if let Some(rest) = text.strip_prefix(prefix) {
            return (tool_name.to_string(), rest.to_string());
        }
    }

    // Fallback: try to find a colon separator
    if let Some(colon_pos) = text.find(": ") {
        let label = &text[..colon_pos];
        let args = &text[colon_pos + 2..];
        // Use the label as the tool name (capitalize first letter)
        let tool = if label.is_empty() {
            "Tool".to_string()
        } else {
            let mut chars = label.chars();
            match chars.next() {
                None => "Tool".to_string(),
                Some(first) => {
                    let rest: String = chars.collect();
                    format!("{}{}", first.to_uppercase(), rest)
                }
            }
        };
        return (tool, args.to_string());
    }

    // Last resort: entire text is the tool name
    ("Tool".to_string(), text.to_string())
}

/// Extract a compact args summary from a ContentBlock JSON args string.
pub(super) fn parse_content_block_args(tool_name: &str, args_str: &str) -> String {
    let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) else {
        return args_str.to_string();
    };
    match tool_name {
        "Read" | "Write" | "Edit" => args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or(args_str)
            .to_string(),
        "Bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or(args_str);
            if cmd.len() > 80 { format!("{}...", &cmd[..77]) } else { cmd.to_string() }
        }
        "Grep" => args.get("pattern").and_then(|v| v.as_str()).unwrap_or(args_str).to_string(),
        "Glob" => args.get("pattern").and_then(|v| v.as_str()).unwrap_or(args_str).to_string(),
        "Task" | "delegate_to_agent" => args
            .get("agent_id")
            .or_else(|| args.get("agent"))
            .and_then(|v| v.as_str())
            .unwrap_or(args_str)
            .to_string(),
        "WebFetch" => args.get("url").and_then(|v| v.as_str()).unwrap_or(args_str).to_string(),
        "WebSearch" => args.get("query").and_then(|v| v.as_str()).unwrap_or(args_str).to_string(),
        _ => {
            if args_str.len() > 60 { format!("{}...", &args_str[..57]) } else { args_str.to_string() }
        }
    }
}

/// Strip "Step N: " prefix from a plan item title, returning the rest.
fn strip_step_prefix(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("Step ") {
        if let Some(colon_pos) = rest.find(": ") {
            let num_part = &rest[..colon_pos];
            if num_part.chars().all(|c| c.is_ascii_digit()) {
                return &rest[colon_pos + 2..];
            }
        }
    }
    s
}

/// Deduplicate plan items: normalize by stripping "Step N: " prefixes,
/// then keep only the first occurrence of each unique title.
pub(super) fn dedup_plan_items(items: Vec<PlanDisplayItem>) -> Vec<PlanDisplayItem> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            let normalized = strip_step_prefix(&item.title).to_string();
            seen.insert(normalized)
        })
        .collect()
}
