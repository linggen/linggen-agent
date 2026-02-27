use super::App;

impl App {
    /// Check if a message should be hidden (matches WebUI's shouldHideInternalChatMessage).
    pub(super) fn should_hide_internal_message(text: &str) -> bool {
        if text.starts_with("Starting autonomous loop for task:") {
            return true;
        }
        // Hide raw tool observation messages: "Tool Bash: ...", "Tool Read: ..."
        if let Some(rest) = text.strip_prefix("Tool ") {
            if rest.chars().next().map_or(false, |c| c.is_alphanumeric()) {
                if rest.contains(':') {
                    return true;
                }
            }
        }
        if text.starts_with("Used tool:") {
            return true;
        }
        if text.starts_with("Delegated task:") {
            return true;
        }
        false
    }

    /// Check if text is a transient status line (matches WebUI's isStatusLineText).
    pub(super) fn is_status_line_text(text: &str) -> bool {
        text == "Thinking..."
            || text == "Thinking"
            || text.starts_with("Thinking (")
            || text == "Model loading..."
            || text.starts_with("Loading model:")
            || text == "Running"
            || text == "Reading file..."
            || text.starts_with("Reading file:")
            || text == "Writing file..."
            || text.starts_with("Writing file:")
    }

    /// Strip internal JSON (tool calls, actions) from a message.
    pub(super) fn strip_internal_json(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if !trimmed.contains('{') {
            return trimmed.to_string();
        }

        let mut result = String::new();
        let bytes = trimmed.as_bytes();
        let mut pos = 0;

        while pos < bytes.len() {
            if bytes[pos] == b'{' {
                if let Some(end) = Self::find_json_object_end(trimmed, pos) {
                    let json_slice = &trimmed[pos..end];
                    let is_internal = (json_slice.contains("\"name\"")
                        && json_slice.contains("\"args\""))
                        || json_slice.contains("\"type\"");
                    if is_internal {
                        pos = end;
                        continue;
                    }
                }
                result.push('{');
                pos += 1;
            } else {
                result.push(bytes[pos] as char);
                pos += 1;
            }
        }

        result.trim().to_string()
    }

    pub(super) fn find_json_object_end(s: &str, start: usize) -> Option<usize> {
        let bytes = s.as_bytes();
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;

        for i in start..bytes.len() {
            let c = bytes[i];
            if escape {
                escape = false;
                continue;
            }
            if in_string {
                if c == b'\\' {
                    escape = true;
                } else if c == b'"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
