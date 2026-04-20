//! Direct read/edit endpoints for memory files under `~/.linggen/memory/`.
//!
//! The memory skill's dashboard buttons (edit/delete per fact) used to send
//! a hidden chat message, which spun up the LLM just to delete one line — slow,
//! costs tokens, and can misread. These endpoints let the UI mutate a memory
//! file directly with a single HTTP call.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use super::ServerState;

/// The five known memory filenames. Any other filename is rejected — this
/// endpoint is not a general-purpose file editor.
const ALLOWED_FILES: &[&str] = &[
    "user_info.md",
    "user_feedback.md",
    "agent_done_week.md",
    "agent_done_month.md",
    "agent_done_year.md",
];

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteFactRequest {
    pub file: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EditFactRequest {
    pub file: String,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Serialize)]
struct MutateResponse {
    ok: bool,
    removed_line: Option<String>,
    matched: usize,
}

pub(crate) async fn delete_fact_api(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<DeleteFactRequest>,
) -> impl IntoResponse {
    let path = match resolve_memory_path(&req.file) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    match mutate_memory_file(&path, |body| remove_matching_line(body, &req.text)) {
        Ok((removed, matched)) => Json(MutateResponse {
            ok: true,
            removed_line: removed,
            matched,
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(crate) async fn edit_fact_api(
    State(_state): State<Arc<ServerState>>,
    Json(req): Json<EditFactRequest>,
) -> impl IntoResponse {
    let path = match resolve_memory_path(&req.file) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    if req.new_text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "new_text is empty".to_string()).into_response();
    }

    match mutate_memory_file(&path, |body| {
        replace_matching_line(body, &req.old_text, &req.new_text)
    }) {
        Ok((replaced, matched)) => Json(MutateResponse {
            ok: true,
            removed_line: replaced,
            matched,
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_memory_path(file: &str) -> Result<PathBuf, String> {
    if !ALLOWED_FILES.contains(&file) {
        return Err(format!(
            "Unknown memory file '{file}'. Allowed: {:?}",
            ALLOWED_FILES
        ));
    }
    let home = dirs::home_dir().ok_or_else(|| "No home directory".to_string())?;
    Ok(home.join(".linggen").join("memory").join(file))
}

/// Read the file, split off the frontmatter, apply `mutate_body` to the body,
/// then atomically write the file back preserving the frontmatter. Returns
/// whatever the mutator returned (typically the removed/replaced line + match count).
fn mutate_memory_file<F, T>(path: &std::path::Path, mutate_body: F) -> Result<T, String>
where
    F: FnOnce(&str) -> (String, T),
{
    let original = std::fs::read_to_string(path).map_err(|e| format!("read {:?}: {e}", path))?;
    let (frontmatter, body) = split_frontmatter(&original);
    let (new_body, result) = mutate_body(body);
    let new_content = if frontmatter.is_empty() {
        new_body
    } else {
        format!("{frontmatter}\n{new_body}")
    };

    // Atomic write: temp file + rename, so a crash mid-write doesn't corrupt.
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, &new_content).map_err(|e| format!("write tmp {:?}: {e}", tmp))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {:?}: {e}", path))?;

    Ok(result)
}

/// Split a YAML-frontmatter markdown doc at the second `---` line.
/// Returns `(frontmatter_incl_delimiters, body)`. If no frontmatter, returns ("", whole).
fn split_frontmatter(content: &str) -> (String, &str) {
    if !content.starts_with("---") {
        return (String::new(), content);
    }
    // Find the closing `---` on its own line after the first.
    let after_first = &content[3..]; // skip leading ---
    if let Some(end_rel) = after_first.find("\n---") {
        let end_abs = 3 + end_rel + 4; // include the closing ---
        // Advance past the newline after the closing ---
        let mut tail_start = end_abs;
        if content.as_bytes().get(tail_start) == Some(&b'\n') {
            tail_start += 1;
        }
        let frontmatter = content[..tail_start.saturating_sub(1)].to_string();
        let body = &content[tail_start..];
        return (frontmatter, body);
    }
    (String::new(), content)
}

/// Remove the first line in `body` whose trimmed form matches `- <text>` or
/// `- <text> (YYYY-MM-DD)`. Returns the updated body + the removed line (if any)
/// + the number of candidate matches (to detect ambiguity).
fn remove_matching_line(body: &str, fact_text: &str) -> (String, (Option<String>, usize)) {
    let needle = fact_text.trim();
    let mut out = String::with_capacity(body.len());
    let mut removed: Option<String> = None;
    let mut matched = 0usize;
    for line in body.split_inclusive('\n') {
        if removed.is_none() && line_matches_fact(line, needle) {
            matched += 1;
            removed = Some(line.trim_end_matches('\n').to_string());
            continue; // skip writing this line
        } else if line_matches_fact(line, needle) {
            matched += 1;
            out.push_str(line);
        } else {
            out.push_str(line);
        }
    }
    (out, (removed, matched))
}

/// Replace the first matching fact line with `- <new_text>` (preserving a
/// trailing `(YYYY-MM-DD)` date if the original had one).
fn replace_matching_line(
    body: &str,
    old_text: &str,
    new_text: &str,
) -> (String, (Option<String>, usize)) {
    let needle = old_text.trim();
    let new_trimmed = new_text.trim();
    let mut out = String::with_capacity(body.len() + new_trimmed.len());
    let mut replaced: Option<String> = None;
    let mut matched = 0usize;
    for line in body.split_inclusive('\n') {
        if replaced.is_none() && line_matches_fact(line, needle) {
            matched += 1;
            // Preserve a trailing (YYYY-MM-DD) date if the original had one.
            let trailing_date = extract_trailing_date(line);
            let indent: String = line.chars().take_while(|c| *c == ' ').collect();
            let new_line = match trailing_date {
                Some(d) => format!("{indent}- {new_trimmed} {d}\n"),
                None => format!("{indent}- {new_trimmed}\n"),
            };
            replaced = Some(line.trim_end_matches('\n').to_string());
            out.push_str(&new_line);
        } else if line_matches_fact(line, needle) {
            matched += 1;
            out.push_str(line);
        } else {
            out.push_str(line);
        }
    }
    (out, (replaced, matched))
}

/// True when `line` is a bullet whose body (ignoring the leading `- ` and any
/// trailing `(YYYY-MM-DD)`) equals `needle` after normalizing whitespace.
fn line_matches_fact(line: &str, needle: &str) -> bool {
    let t = line.trim();
    let Some(rest) = t.strip_prefix("- ") else {
        return false;
    };
    let rest = strip_trailing_date(rest);
    normalize_ws(rest) == normalize_ws(needle)
}

fn strip_trailing_date(s: &str) -> &str {
    let s = s.trim_end();
    // Look for `(YYYY-MM-DD)` suffix.
    if let Some(open) = s.rfind(" (") {
        let tail = &s[open + 2..];
        if tail.ends_with(')') {
            let inner = &tail[..tail.len() - 1];
            if is_date(inner) {
                return s[..open].trim_end();
            }
        }
    }
    s
}

fn extract_trailing_date(line: &str) -> Option<String> {
    let s = line.trim_end_matches('\n').trim_end();
    let open = s.rfind(" (")?;
    let tail = &s[open + 2..];
    if !tail.ends_with(')') {
        return None;
    }
    let inner = &tail[..tail.len() - 1];
    if is_date(inner) {
        Some(format!("({inner})"))
    } else {
        None
    }
}

fn is_date(s: &str) -> bool {
    // YYYY-MM-DD
    if s.len() != 10 {
        return false;
    }
    let b = s.as_bytes();
    b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..].iter().all(|c| c.is_ascii_digit())
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_fact_with_and_without_date() {
        assert!(line_matches_fact("- Prefers dark mode (2026-04-17)\n", "Prefers dark mode"));
        assert!(line_matches_fact("- Prefers dark mode\n", "Prefers dark mode"));
        assert!(!line_matches_fact("- Prefers light mode\n", "Prefers dark mode"));
    }

    #[test]
    fn remove_matching_line_strips_one() {
        let body = "## Identity\n\n- Lives in Vancouver (2026-04-17)\n- Prefers dark mode\n";
        let (out, (removed, matched)) =
            remove_matching_line(body, "Prefers dark mode");
        assert_eq!(matched, 1);
        assert_eq!(removed.as_deref(), Some("- Prefers dark mode"));
        assert!(!out.contains("Prefers dark mode"));
        assert!(out.contains("Lives in Vancouver"));
    }

    #[test]
    fn replace_preserves_date() {
        let body = "- Lives in Vancouver (2026-04-17)\n";
        let (out, (replaced, _)) =
            replace_matching_line(body, "Lives in Vancouver", "Lives in Toronto");
        assert!(replaced.is_some());
        assert_eq!(out, "- Lives in Toronto (2026-04-17)\n");
    }

    #[test]
    fn split_frontmatter_basic() {
        let doc = "---\nname: user_info\n---\n\n## Identity\n- x\n";
        let (fm, body) = split_frontmatter(doc);
        assert!(fm.contains("name: user_info"));
        assert!(body.starts_with("\n## Identity"));
    }
}
