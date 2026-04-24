//! Capability registry — canonical tool contracts implemented by
//! pluggable skills.
//!
//! A **capability** is a named set of tools the engine defines once and
//! any number of skills implement. Skills declare `provides: [<name>]`
//! and an `implements:` block mapping each tool to its HTTP endpoint on
//! the skill's daemon. The engine owns: tool names, argument schemas,
//! descriptions, and permission tiers. Skills own: the backend URL + the
//! wire path per tool.
//!
//! This matches the "canonical contract" rule in `doc/memory-spec.md` —
//! swapping providers preserves the model's tool surface identically.
//! Capability tools appear in the model's tool list (with schemas) only
//! when an active `provides: [<name>]` skill is installed and the
//! session's prompt profile opts into the capability (owner sessions do;
//! consumer/mission sessions don't).
//!
//! Adding a new capability = appending one more entry to `CAPABILITIES`.
//! Adding a new tool to an existing capability = appending one more
//! `CapabilityTool` + updating any skill's `implements:` block that
//! claims to cover the whole capability. The engine validates neither
//! shape today; future work: an engine-side check that each active
//! provider's `implements:` covers every declared tool in the capability.

use super::permission::PermissionMode;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// One tool in a capability's canonical contract.
#[derive(Debug, Clone)]
pub struct CapabilityTool {
    pub name: String,
    pub description: String,
    pub tier: PermissionMode,
    /// JSON-Schema for the tool's arguments. Used directly for native
    /// tool calling; converted to the legacy "args: {k: 'type'}" shape
    /// for legacy JSON-action format.
    pub args_schema: Value,
}

/// A named capability — a set of tools the engine defines, a skill
/// implements.
#[derive(Debug, Clone)]
pub struct Capability {
    pub name: String,
    pub tools: Vec<CapabilityTool>,
}

/// All capabilities the engine knows about. Skills must use these exact
/// tool names; schemas here are the truth the model sees.
pub static CAPABILITIES: LazyLock<Vec<Capability>> = LazyLock::new(build_capabilities);

fn build_capabilities() -> Vec<Capability> {
    vec![memory_capability()]
}

// ── Memory capability ──────────────────────────────────────────────────

fn memory_capability() -> Capability {
    Capability {
        name: "memory".to_string(),
        tools: vec![
            CapabilityTool {
                name: "Memory_add".to_string(),
                description: "Store a new memory fact. Memory is how the agent grows up — a deepening model of WHO the user is, not a log of what was done. Save only if a future session (any project, months from now) would make better predictions about this user because the fact exists. Primary types to emit: `fact` (user identity / goals), `preference` (commitment-language behavioral rule), `decision` (cross-project reasoning), `learned` (cross-project tech gotcha). Deprecated (emit only in narrow cases): `tried` / `fixed` / `built` — project-specific bug fixes, daily activity, and single-session attempts belong in git log, not memory. Route project-specific implementation detail to suggest_claude_md at the skill level, never via this tool. Auto-dedups server-side: near-duplicates merge instead of inserting (returns `{action: \"merged\", similarity, previous_id, fact}`). Pass `skip_dedup: true` only from a scan pipeline running its own dedup pass.".to_string(),
                tier: PermissionMode::Edit,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "content":    {"type": "string", "description": "The fact text. Self-contained; include scoping conditions inline if they matter."},
                        "contexts":   {"type": "array", "items": {"type": "string"}, "description": "Scope tags (e.g. [\"code/linggen\", \"trip-japan-2026\"]). Free-form; N:M with facts."},
                        "tags":       {"type": "array", "items": {"type": "string"}, "description": "Free-form metadata with prefix convention (e.g. \"topic:ui\", \"person:maria\")."},
                        "type":       {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Canonical fact type. Prefer `fact` / `preference` / `decision` / `learned` for new writes. `tried` / `fixed` / `built` are deprecated — emit only for trajectory-level patterns, cross-project diagnostic wisdom, or named shippable artifacts tied to user identity. See the memory skill's extractor-prompt.md for the full routing rules."},
                        "from":       {"type": "string", "enum": ["user", "agent", "derived"], "description": "Origin. Defaults to derived."},
                        "outcome":    {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "Only meaningful for `tried` / `fixed` (deprecated action-flavored types). Omit for `fact` / `preference` / `decision` / `learned`."},
                        "cwd":        {"type": "string", "description": "Working directory where the fact was produced."},
                        "occurred_at":{"type": "string", "description": "RFC-3339 timestamp of the described event. Omit if unknown."},
                        "source_session":{"type": "string", "description": "Opaque session id the fact was extracted from."},
                        "skip_dedup": {"type": "boolean", "description": "Skip server-side merge-into-near-duplicate. Default false. Set to true when the caller is running its own dedup pass."}
                    },
                    "required": ["content"]
                }),
            },
            CapabilityTool {
                name: "Memory_get".to_string(),
                description: "Fetch a single fact by id.".to_string(),
                tier: PermissionMode::Read,
                args_schema: json!({
                    "type": "object",
                    "properties": {"id": {"type": "string", "description": "Fact UUID."}},
                    "required": ["id"]
                }),
            },
            CapabilityTool {
                name: "Memory_search".to_string(),
                description: "Semantic search across stored facts. Use when the query is fuzzy or you want relevance ranking. No longer needed as a dedup precheck — Memory_add handles that server-side.".to_string(),
                tier: PermissionMode::Read,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "query":    {"type": "string", "description": "Natural-language query — describe what you're looking for."},
                        "contexts": {"type": "array", "items": {"type": "string"}, "description": "Narrow to these scope tags (AND semantics)."},
                        "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                        "from":     {"type": "string", "enum": ["user", "agent", "derived"]},
                        "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"]},
                        "since":    {"type": "string", "description": "RFC-3339 lower bound on effective timestamp. Omit to skip."},
                        "limit":    {"type": "integer", "description": "Max rows to return. Defaults to 10."}
                    },
                    "required": ["query"]
                }),
            },
            CapabilityTool {
                name: "Memory_list".to_string(),
                description: "Browse facts without semantic ranking — filter-only. Use for audits or exact-match enumeration.".to_string(),
                tier: PermissionMode::Read,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "contexts": {"type": "array", "items": {"type": "string"}},
                        "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                        "from":     {"type": "string", "enum": ["user", "agent", "derived"]},
                        "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"]},
                        "since":    {"type": "string", "description": "RFC-3339 lower bound. Omit to skip."},
                        "until":    {"type": "string", "description": "RFC-3339 upper bound. Omit to skip."},
                        "sort":     {"type": "string", "enum": ["newest", "oldest"], "description": "Defaults to newest."},
                        "limit":    {"type": "integer", "description": "Max rows to return. Defaults to 50."},
                        "offset":   {"type": "integer", "description": "Skip this many rows in sort order. Pairs with limit for pagination."}
                    },
                    "required": []
                }),
            },
            CapabilityTool {
                name: "Memory_update".to_string(),
                description: "Edit fields of an existing fact. Use when the user corrects a recorded fact, or when dedup finds a better phrasing for the same meaning.".to_string(),
                tier: PermissionMode::Edit,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "id":            {"type": "string"},
                        "content":       {"type": "string"},
                        "contexts":      {"type": "array", "items": {"type": "string"}},
                        "tags":          {"type": "array", "items": {"type": "string"}},
                        "type":          {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                        "from":          {"type": "string", "enum": ["user", "agent", "derived"]},
                        "outcome":       {"type": "string", "enum": ["positive", "negative", "neutral"]},
                        "clear_outcome": {"type": "boolean", "description": "Clear the outcome to null."},
                        "cwd":           {"type": "string"},
                        "clear_cwd":     {"type": "boolean", "description": "Clear cwd to null."}
                    },
                    "required": ["id"]
                }),
            },
            CapabilityTool {
                name: "Memory_delete".to_string(),
                description: "Hard-delete a fact by id. Use for dedup (remove stale/contradicted rows before saving a better version) or when the user retracts a fact. Not reversible.".to_string(),
                tier: PermissionMode::Edit,
                args_schema: json!({
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"]
                }),
            },
            CapabilityTool {
                name: "Memory_forget".to_string(),
                description: "Bulk-delete rows by filter. Destructive. ONLY on explicit user request (\"forget everything about X\") and ONLY with a specific filter. Never run during extraction. Refuses empty filters — supply at least one of contexts / type / from / outcome / since / until.".to_string(),
                tier: PermissionMode::Admin,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "contexts": {"type": "array", "items": {"type": "string"}},
                        "type":     {"type": "string"},
                        "from":     {"type": "string"},
                        "outcome":  {"type": "string"},
                        "since":    {"type": "string", "description": "RFC-3339 lower bound. Omit to skip."},
                        "until":    {"type": "string", "description": "RFC-3339 upper bound. Omit to skip."}
                    },
                    "required": []
                }),
            },
        ],
    }
}

// ── Lookup helpers ─────────────────────────────────────────────────────

/// Find the capability that owns the given tool name. Returns the
/// capability's name + a ref to its tool descriptor. The returned
/// references live as long as `CAPABILITIES` — which is a `LazyLock`
/// in static position, i.e. program-lifetime.
pub fn capability_for_tool(tool_name: &str) -> Option<(&'static str, &'static CapabilityTool)> {
    CAPABILITIES.iter().find_map(|cap| {
        cap.tools
            .iter()
            .find(|t| t.name == tool_name)
            .map(|t| (cap.name.as_str(), t))
    })
}

/// Iterate every capability tool across every registered capability.
pub fn all_capability_tools() -> impl Iterator<Item = (&'static str, &'static CapabilityTool)> {
    CAPABILITIES
        .iter()
        .flat_map(|cap| cap.tools.iter().map(move |t| (cap.name.as_str(), t)))
}

/// Permission tier for a capability tool by name.
pub fn tool_tier(tool_name: &str) -> Option<PermissionMode> {
    capability_for_tool(tool_name).map(|(_, t)| t.tier)
}

/// Is this tool name a capability tool (as opposed to a built-in or a
/// skill-unique tool)?
pub fn is_capability_tool(tool_name: &str) -> bool {
    capability_for_tool(tool_name).is_some()
}

/// Render a capability tool's arg schema to the legacy tool-schema
/// format used by `full_tool_schema_entries` — `{"name", "args":{k:"type"},
/// "returns", "notes"}`. The "args" here are compact type strings derived
/// from the JSON Schema (`string` / `string?` / `string[]` / `string[]?`
/// / `integer` / etc.).
pub fn legacy_schema_entry(tool: &CapabilityTool) -> Value {
    let mut args = serde_json::Map::new();
    let required: Vec<String> = tool
        .args_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if let Some(props) = tool.args_schema.get("properties").and_then(|p| p.as_object()) {
        for (name, spec) in props {
            let is_required = required.iter().any(|r| r == name);
            let ty = spec.get("type").and_then(|v| v.as_str()).unwrap_or("string");
            let compact = match ty {
                "array" => {
                    let item_ty = spec
                        .get("items")
                        .and_then(|it| it.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("string");
                    format!("{item_ty}[]")
                }
                other => other.to_string(),
            };
            let with_opt = if is_required {
                compact
            } else {
                format!("{compact}?")
            };
            args.insert(name.clone(), Value::String(with_opt));
        }
    }
    let mut entry = json!({
        "name": tool.name,
        "args": args,
    });
    if !tool.description.is_empty() {
        entry["notes"] = Value::String(tool.description.clone());
    }
    entry
}

/// OpenAI-compatible function schema for native tool calling.
pub fn oai_schema_entry(tool: &CapabilityTool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.args_schema,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_capability_registered_with_seven_tools() {
        let mem = CAPABILITIES
            .iter()
            .find(|c| c.name == "memory")
            .expect("memory capability present");
        assert_eq!(mem.tools.len(), 7);
    }

    #[test]
    fn capability_for_tool_finds_memory_tools() {
        let (cap_name, tool) =
            capability_for_tool("Memory_search").expect("Memory_search is a capability tool");
        assert_eq!(cap_name, "memory");
        assert_eq!(tool.name, "Memory_search");
        assert_eq!(tool.tier, PermissionMode::Read);
    }

    #[test]
    fn capability_for_tool_misses_for_builtins() {
        assert!(capability_for_tool("Read").is_none());
        assert!(capability_for_tool("Bash").is_none());
        assert!(capability_for_tool("Unknown").is_none());
    }

    #[test]
    fn tool_tiers_match_canonical_contract() {
        assert_eq!(tool_tier("Memory_search"), Some(PermissionMode::Read));
        assert_eq!(tool_tier("Memory_list"),   Some(PermissionMode::Read));
        assert_eq!(tool_tier("Memory_get"),    Some(PermissionMode::Read));
        assert_eq!(tool_tier("Memory_add"),    Some(PermissionMode::Edit));
        assert_eq!(tool_tier("Memory_update"), Some(PermissionMode::Edit));
        assert_eq!(tool_tier("Memory_delete"), Some(PermissionMode::Edit));
        assert_eq!(tool_tier("Memory_forget"), Some(PermissionMode::Admin));
    }

    #[test]
    fn legacy_schema_entry_compacts_types() {
        let tool = capability_for_tool("Memory_search").unwrap().1;
        let entry = legacy_schema_entry(tool);
        assert_eq!(entry["name"], "Memory_search");
        assert_eq!(entry["args"]["query"], "string");
        assert_eq!(entry["args"]["contexts"], "string[]?");
        assert_eq!(entry["args"]["limit"], "integer?");
    }

    #[test]
    fn oai_schema_wraps_as_function() {
        let tool = capability_for_tool("Memory_add").unwrap().1;
        let entry = oai_schema_entry(tool);
        assert_eq!(entry["type"], "function");
        assert_eq!(entry["function"]["name"], "Memory_add");
        let required = entry["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v == "content"));
    }
}
