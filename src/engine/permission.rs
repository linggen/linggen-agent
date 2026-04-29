use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

// ===========================================================================
// Permission system — see `doc/permission-spec.md`.
//
// State per session: a list of (path, mode) grants. Effective mode for the
// current cwd is the most-specific matching grant, or `chat` (no tools) if
// nothing covers it. Reads are gated like writes — there is no "reads are
// free" carveout. A short hardcoded deny floor blocks the worst foot-guns
// regardless of mode. No config rule layer.
// ===========================================================================

/// Permission action returned after prompting the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    Deny,
    /// User denied with a custom message to relay to the model.
    DenyWithMessage(String),
}

/// Session permission mode — defines the ceiling of what the agent can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Chat,
    Read,
    Edit,
    Admin,
}

impl Default for PermissionMode {
    fn default() -> Self {
        PermissionMode::Read
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionMode::Chat => write!(f, "chat"),
            PermissionMode::Read => write!(f, "read"),
            PermissionMode::Edit => write!(f, "edit"),
            PermissionMode::Admin => write!(f, "admin"),
        }
    }
}

/// A path-scoped mode grant. The mode covers the path and all its children.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMode {
    pub path: String,
    pub mode: PermissionMode,
}

/// Bash command classification for permission tier mapping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BashClass {
    Read,
    Write,
    Admin,
}

// ---------------------------------------------------------------------------
// SessionPermissions — the only persisted permission state.
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

/// Per-session permission state, persisted to `permission.json`.
///
/// `path_modes[]` is the entire grant table — only explicit user approvals
/// (mode upgrade prompts), mission frontmatter, and skill frontmatter write
/// to it. `interactive` is metadata: false for mission and proxy-consumer
/// sessions so the engine pauses or fails instead of trying to prompt.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default)]
    pub path_modes: Vec<PathMode>,
    /// True for normal user sessions (prompt on permission-needed). False for
    /// mission and proxy-consumer sessions (pause/fail; never prompt).
    #[serde(default = "default_true")]
    pub interactive: bool,
}

impl Default for SessionPermissions {
    fn default() -> Self {
        Self { path_modes: Vec::new(), interactive: true }
    }
}

impl SessionPermissions {
    /// Load from `{session_dir}/permission.json`. Returns default if missing.
    /// Tolerates legacy fields (`policy`, `locked`, `allows`, `denied_sigs`)
    /// — they're ignored, which is the migration.
    pub fn load(session_dir: &Path) -> Self {
        let file = session_dir.join("permission.json");
        if !file.exists() {
            return Self::default();
        }
        match fs::read_to_string(&file) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to parse session permission.json: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("Failed to read session permission.json: {}", e);
                Self::default()
            }
        }
    }

    /// Save to `{session_dir}/permission.json`.
    pub fn save(&self, session_dir: &Path) {
        let file = session_dir.join("permission.json");
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&file, json) {
                    warn!("Failed to write session permission.json: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize session permissions: {}", e),
        }
    }

    /// Add or update a path-mode grant. If a grant for the exact path exists, update it.
    /// Prunes child entries that are now redundant or that conflict with a downgrade.
    pub fn set_path_mode(&mut self, path: &str, mode: PermissionMode) {
        let expanded = expand_tilde(path);

        self.path_modes.retain(|pm| {
            if pm.path == path {
                return true;
            }
            let pm_expanded = expand_tilde(&pm.path);
            let is_child = pm_expanded.starts_with(&expanded)
                && (pm_expanded.len() == expanded.len()
                    || pm_expanded.as_bytes().get(expanded.len()) == Some(&b'/'));
            !is_child
        });

        if let Some(existing) = self.path_modes.iter_mut().find(|pm| pm.path == path) {
            existing.mode = mode;
        } else {
            self.path_modes.push(PathMode {
                path: path.to_string(),
                mode,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Hardcoded deny floor — engine-baked, not user-configurable.
//
// Curated list of patterns that are always blocked regardless of mode. Admin
// mode does NOT bypass this. See `doc/permission-spec.md` §"Hardcoded deny
// floor".
// ---------------------------------------------------------------------------

/// True if the given bash command matches the hardcoded deny floor.
pub fn is_hardcoded_deny(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Forkbomb pattern :(){:|:&};: — also tolerates whitespace variants
    // (`:() { :|:& };:`).
    let dewhitespaced: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if dewhitespaced.contains(":(){:|:&}") {
        return true;
    }

    // Walk each command segment in a compound command and test independently.
    for segment in split_command_segments(trimmed) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        if segment_is_hardcoded_deny(seg) {
            return true;
        }
    }
    false
}

fn segment_is_hardcoded_deny(seg: &str) -> bool {
    let tokens: Vec<&str> = seg.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }
    let program = tokens[0];

    // sudo / sudoedit — privilege escalation, never allowed.
    if program == "sudo" || program == "sudoedit" {
        return true;
    }

    // mkfs.* — filesystem creation on a device.
    if program == "mkfs" || program.starts_with("mkfs.") {
        return true;
    }

    // dd of=/dev/{disk,sd*,nvme*,hd*,mmcblk*} — direct disk overwrite.
    if program == "dd" {
        for arg in &tokens[1..] {
            if let Some(target) = arg.strip_prefix("of=") {
                if dd_target_is_blockdev(target) {
                    return true;
                }
            }
        }
    }

    // rm -rf / and rm -rf /* — whole-disk wipe.
    if program == "rm" {
        let has_recursive = tokens[1..].iter().any(|t| {
            *t == "-rf" || *t == "-fr" || *t == "-Rf" || *t == "-fR" || *t == "--recursive"
                || *t == "-r" || *t == "-R"
        });
        let has_force = tokens[1..].iter().any(|t| {
            *t == "-f" || *t == "-rf" || *t == "-fr" || *t == "-Rf" || *t == "-fR" || *t == "--force"
        });
        if has_recursive && has_force {
            for arg in &tokens[1..] {
                if rm_target_is_root(arg) {
                    return true;
                }
            }
        }
    }

    // chown -R … / and chmod -R … / — root-tree ownership/mode bombs.
    if program == "chown" || program == "chmod" {
        let has_recursive = tokens[1..]
            .iter()
            .any(|t| *t == "-R" || *t == "--recursive");
        if has_recursive {
            if let Some(last) = tokens.last() {
                if rm_target_is_root(last) {
                    return true;
                }
            }
        }
    }

    false
}

fn dd_target_is_blockdev(target: &str) -> bool {
    if let Some(rest) = target.strip_prefix("/dev/") {
        return rest.starts_with("disk")
            || rest.starts_with("sd")
            || rest.starts_with("nvme")
            || rest.starts_with("hd")
            || rest.starts_with("mmcblk");
    }
    false
}

fn rm_target_is_root(arg: &str) -> bool {
    matches!(arg, "/" | "/*" | "/.*" | "~" | "~/" | "~/*" | "/.")
}

/// Split a compound command into segments by `;`, `&&`, `||`, `|`.
fn split_command_segments(cmd: &str) -> Vec<String> {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        let split = match b {
            b';' => Some(1usize),
            b'|' => Some(if i + 1 < len && bytes[i + 1] == b'|' { 2 } else { 1 }),
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => Some(2),
            _ => None,
        };
        if let Some(advance) = split {
            segments.push(cmd[start..i].to_string());
            i += advance;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < len {
        segments.push(cmd[start..].to_string());
    }
    segments
}

// ---------------------------------------------------------------------------
// Bash command classifier
// ---------------------------------------------------------------------------

fn is_compound_command(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        match bytes[i] {
            b';' => return true,
            b'|' => return true,
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => return true,
            b'`' => return true,
            b'$' if i + 1 < len && bytes[i + 1] == b'(' => return true,
            _ => {}
        }
    }
    false
}

fn has_output_redirect(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            if i > 0 && bytes[i - 1] == b'-' {
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                continue;
            }
            let after = cmd[i..].trim_start_matches('>').trim();
            if after.starts_with("/dev/null") {
                continue;
            }
            return true;
        }
    }
    false
}

/// Classify a bash command into read/write/admin tier.
pub fn classify_bash_command(cmd: &str) -> BashClass {
    if is_compound_command(cmd) {
        return classify_compound_command(cmd);
    }

    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return BashClass::Admin;
    }

    let program = tokens[0];
    let subcommand = tokens.get(1).copied().unwrap_or("");

    if has_output_redirect(cmd) {
        let base = classify_single_command(program, subcommand);
        return if base == BashClass::Admin {
            BashClass::Admin
        } else {
            BashClass::Write
        };
    }

    classify_single_command(program, subcommand)
}

fn classify_single_command(program: &str, subcommand: &str) -> BashClass {
    const READ_PROGRAMS: &[&str] = &[
        "ls", "cat", "head", "tail", "less", "more", "wc", "file", "stat", "du", "df",
        "pwd", "env", "printenv", "echo", "printf", "which", "whereis", "type",
        "find", "grep", "rg", "ag", "ack", "fd", "tree", "bat", "jq", "yq",
        "uname", "hostname", "date", "id", "whoami", "realpath", "dirname", "basename",
        "ping", "dig", "nslookup", "host", "test", "true", "false", "seq", "sort",
        "uniq", "tr", "cut", "paste", "diff", "comm",
        // Read-only system diagnostics (used by sys-doctor and similar inspection
        // skills). These programs only display info — they don't mutate state.
        "uptime", "sw_vers", "vm_stat", "netstat", "ifconfig", "ipconfig", "scutil",
        "ps", "lsof", "vmmap", "iostat",
        // macOS system / security inspection. Bare invocations are read-only;
        // mutating forms (sysctl -w, csrutil disable, spctl --add, fdesetup
        // enable, etc.) all require sudo, which the hardcoded deny floor blocks.
        "sysctl", "spctl", "csrutil", "fdesetup", "socketfilterfw", "systemsetup",
    ];

    const GIT_READ: &[&str] = &[
        "status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse",
        "blame", "stash", "describe", "shortlog", "ls-files", "ls-tree",
    ];

    const CARGO_READ: &[&str] = &["check", "clippy", "doc", "metadata", "tree", "verify-project"];
    const NPM_READ: &[&str] = &["list", "ls", "outdated", "view", "info", "audit", "why", "explain"];
    const PIP_READ: &[&str] = &["list", "show", "freeze", "check"];
    const GO_READ: &[&str] = &["vet", "list", "doc", "env", "version"];

    const ADMIN_PROGRAMS: &[&str] = &[
        "rm", "sudo", "su", "kill", "killall", "pkill",
        "chmod", "chown", "chgrp",
        "podman", "systemctl", "launchctl", "service",
        "mount", "umount", "mkfs", "fdisk", "dd",
        "apt", "apt-get", "yum", "dnf", "pacman",
        "reboot", "shutdown", "halt", "poweroff",
        "iptables", "ufw", "firewall-cmd",
        "crontab", "at",
    ];

    const WRITE_PROGRAMS: &[&str] = &[
        "mkdir", "cp", "mv", "touch", "sed", "awk", "patch",
        "ln", "install", "rsync", "tee",
    ];

    const GIT_WRITE: &[&str] = &[
        "add", "commit", "push", "pull", "merge", "rebase", "checkout", "switch",
        "fetch", "clone", "init", "reset", "cherry-pick", "am", "apply",
    ];

    const CARGO_WRITE: &[&str] = &["build", "test", "run", "fmt", "install", "publish", "bench"];
    const NPM_WRITE: &[&str] = &["install", "ci", "run", "start", "test", "build", "publish", "exec"];
    const PIP_WRITE: &[&str] = &["install", "uninstall"];
    const GO_WRITE: &[&str] = &["build", "test", "run", "install", "get", "mod"];

    // brew and docker have read subcommands worth carving out so inspection
    // skills (sys-doctor) don't have to admin-prompt for `brew list` / `docker images`.
    const BREW_READ: &[&str] = &["list", "info", "search", "outdated", "deps", "leaves",
                                  "doctor", "config", "--version", "-v", "--prefix", "tap-info"];
    const DOCKER_READ: &[&str] = &["ps", "images", "logs", "inspect", "version", "info",
                                    "system", "history", "port", "top", "stats", "diff", "events"];

    if ADMIN_PROGRAMS.contains(&program) {
        return BashClass::Admin;
    }
    if READ_PROGRAMS.contains(&program) {
        return BashClass::Read;
    }
    if WRITE_PROGRAMS.contains(&program) {
        return BashClass::Write;
    }

    match program {
        "git" => {
            if GIT_READ.contains(&subcommand) { BashClass::Read }
            else if GIT_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "cargo" => {
            if CARGO_READ.contains(&subcommand) { BashClass::Read }
            else if CARGO_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "npm" | "npx" | "yarn" | "pnpm" => {
            if NPM_READ.contains(&subcommand) { BashClass::Read }
            else if NPM_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "pip" | "pip3" => {
            if PIP_READ.contains(&subcommand) { BashClass::Read }
            else if PIP_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "go" => {
            if GO_READ.contains(&subcommand) { BashClass::Read }
            else if GO_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "python" | "python3" | "node" => {
            if subcommand == "--version" || subcommand == "--help" || subcommand == "-V" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "curl" => {
            if subcommand == "-I" || subcommand == "--head" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "wget" => {
            if subcommand == "--spider" { BashClass::Read } else { BashClass::Admin }
        }
        "make" | "cmake" | "ninja" | "mvn" | "gradle" | "pytest" | "jest" | "vitest" => {
            BashClass::Write
        }
        "brew" => {
            if BREW_READ.contains(&subcommand) { BashClass::Read } else { BashClass::Admin }
        }
        "docker" | "podman" => {
            if DOCKER_READ.contains(&subcommand) { BashClass::Read } else { BashClass::Admin }
        }
        _ => BashClass::Admin,
    }
}

fn classify_compound_command(cmd: &str) -> BashClass {
    let mut highest = BashClass::Read;
    let segments: Vec<&str> = cmd
        .split(|c: char| c == ';' || c == '|' || c == '&')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for segment in segments {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let program = tokens[0];
        let sub = tokens.get(1).copied().unwrap_or("");
        let class = classify_single_command(program, sub);
        if class > highest {
            highest = class.clone();
        }
        if highest == BashClass::Admin {
            return BashClass::Admin;
        }
    }
    highest
}

// ---------------------------------------------------------------------------
// Effective mode lookup
// ---------------------------------------------------------------------------

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Tilde-expand and resolve symlinks. On macOS, `/tmp` and `/private/tmp` are
/// the same directory but differ as strings — the engine's `cwd()` returns
/// the canonical `/private/tmp`, while `session.yaml.cwd` keeps the typed
/// `/tmp`. Without canonicalizing both sides at lookup, a grant on one form
/// fails to cover a target on the other.
///
/// Falls back to the tilde-expanded path when canonicalize fails (e.g. the
/// leaf doesn't exist yet — common for Write to a new file). In that case
/// we walk up to the nearest existing ancestor, canonicalize it, and append
/// the missing tail so `/tmp/new-file.txt` still normalizes to
/// `/private/tmp/new-file.txt`.
fn normalize_path(path: &str) -> String {
    let expanded = expand_tilde(path);
    let mut p = std::path::PathBuf::from(&expanded);
    if let Ok(c) = p.canonicalize() {
        return c.to_string_lossy().to_string();
    }
    // Walk up until we find an existing ancestor we can canonicalize.
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while let Some(parent) = p.parent().map(|x| x.to_path_buf()) {
        if let Some(name) = p.file_name() {
            suffix.push(name.to_os_string());
        }
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Ok(c) = parent.canonicalize() {
            let mut out = c;
            for n in suffix.iter().rev() {
                out.push(n);
            }
            return out.to_string_lossy().to_string();
        }
        if parent == p {
            break;
        }
        p = parent;
    }
    expanded
}

/// Find the effective permission mode for a target path by checking path_modes.
/// Returns the mode from the most specific (longest) matching path.
/// Returns `None` if no grant covers the target — caller treats as `chat`.
pub fn effective_mode_for_path(path_modes: &[PathMode], target: &Path) -> Option<PermissionMode> {
    let target_str = normalize_path(&target.to_string_lossy());
    let mut best: Option<(&PathMode, usize)> = None;

    for pm in path_modes {
        let grant_path = normalize_path(&pm.path);
        if target_str.starts_with(&grant_path)
            && (target_str.len() == grant_path.len()
                || grant_path == "/"
                || target_str.as_bytes().get(grant_path.len()) == Some(&b'/'))
        {
            let specificity = grant_path.len();
            if best.is_none() || specificity > best.unwrap().1 {
                best = Some((pm, specificity));
            }
        }
    }
    best.map(|(pm, _)| pm.mode)
}

// ---------------------------------------------------------------------------
// Tool tier mapping
// ---------------------------------------------------------------------------

/// Map a tool name to its permission mode requirement.
pub fn tool_action_tier(tool: &str) -> PermissionMode {
    match tool {
        "Read" | "Glob" | "Grep" | "WebSearch" | "WebFetch" | "capture_screenshot"
        | "EnterPlanMode" | "ExitPlanMode" | "UpdatePlan" | "AskUser" => PermissionMode::Read,
        "Write" | "Edit" => PermissionMode::Edit,
        _ => {
            if let Some(tier) = crate::engine::capabilities::tool_tier(tool) {
                return tier;
            }
            PermissionMode::Admin
        }
    }
}

/// Parse the `tier:` string from a skill tool's manifest into a `PermissionMode`.
pub fn parse_skill_tier(tier: &str) -> Option<PermissionMode> {
    match tier {
        "read" => Some(PermissionMode::Read),
        "edit" => Some(PermissionMode::Edit),
        "admin" => Some(PermissionMode::Admin),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Permission check
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum PermissionCheckResult {
    Allowed,
    Blocked(String),
    NeedsPrompt(PromptKind),
}

/// What kind of prompt to show. With the simplified model there is only one
/// kind — every permission-needed event is a request to upgrade the mode on
/// the target path (or a parent path for files outside cwd).
#[derive(Debug)]
pub enum PromptKind {
    ExceedsCeiling {
        target_mode: PermissionMode,
        path: String,
        tool_summary: String,
    },
}

/// The main permission check.
///
/// The caller (tool_exec) decides what to do with `NeedsPrompt`: prompt the
/// user if `session_perms.interactive`, otherwise treat as permission-needed
/// (pause/fail) for missions and consumer sessions.
pub fn check_permission(
    tool: &str,
    bash_command: Option<&str>,
    file_path: Option<&str>,
    session_cwd: &Path,
    session_perms: &SessionPermissions,
    action_tier_override: Option<PermissionMode>,
) -> PermissionCheckResult {
    // 0a. Hardcoded deny floor (admin mode does not bypass).
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            if is_hardcoded_deny(cmd) {
                return PermissionCheckResult::Blocked(
                    "Command blocked by safety floor (sudo, rm -rf /, mkfs, etc.)".to_string(),
                );
            }
        }
    }

    // 0b. Skill is a navigation primitive — always allowed regardless of mode.
    // Skills that need elevated permissions request them at activation time
    // through their own SKILL.md `permission:` block. Without this bypass, a
    // chat-mode session can never reach a skill via natural language because
    // the model can't call `Skill` to dispatch to it.
    if tool == "Skill" {
        return PermissionCheckResult::Allowed;
    }

    // Chat mode is the lowest tier — any concrete tool's action_tier
    // (Read/Edit/Admin) exceeds it, so step 4 below produces an
    // ExceedsCeiling prompt offering to switch to the needed mode. We
    // intentionally do NOT short-circuit a Chat grant to a hard block:
    // when the user explicitly picks chat and then asks the agent to run
    // something, they want to be asked to upgrade, not silently denied.

    // 1. Classify action tier. Skill-declared override wins over the built-in table.
    let action_tier = if let Some(override_tier) = action_tier_override {
        override_tier
    } else if tool == "Bash" {
        match bash_command {
            Some(cmd) => match classify_bash_command(cmd) {
                BashClass::Read => PermissionMode::Read,
                BashClass::Write => PermissionMode::Edit,
                BashClass::Admin => PermissionMode::Admin,
            },
            None => PermissionMode::Admin,
        }
    } else {
        tool_action_tier(tool)
    };

    // 2. Bash-specific path gating: if the command contains explicit absolute
    // or tilde-prefixed path args, each must be covered at action_tier.
    // Without this, `bash ls /B` from a session with read-on-/A would pass
    // because step 3 only consulted cwd's tier — but the command's actual
    // reach is /B, not /A. cwd is just where the shell starts; we gate on
    // what the command actually touches.
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            let path_args = extract_command_paths(cmd);
            if !path_args.is_empty() {
                for path_arg in &path_args {
                    let arg_path = expand_path_arg(path_arg);
                    let mode = effective_mode_for_path(&session_perms.path_modes, &arg_path);
                    if mode.map_or(true, |m| action_tier > m) {
                        let grant_path = grant_path_for_prompt(tool, &arg_path, session_cwd);
                        let path_str = display_path(&grant_path);
                        return PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
                            target_mode: action_tier,
                            path: path_str,
                            tool_summary: format!("{} {}", tool, cmd),
                        });
                    }
                }
                // Every explicit path arg is covered at action_tier — allowed.
                return PermissionCheckResult::Allowed;
            }
            // No path args: fall through to the cwd check below — `ls`,
            // `cargo build`, etc. operate in cwd, so cwd's tier is the gate.
        }
    }

    // 3. Resolve target path for non-Bash tools (or Bash without path args).
    // Tools without an explicit file_path use cwd.
    let target_path = file_path
        .map(|fp| {
            if fp == "~" {
                dirs::home_dir().unwrap_or_else(|| PathBuf::from(fp))
            } else if let Some(rest) = fp.strip_prefix("~/") {
                dirs::home_dir()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| PathBuf::from(fp))
            } else {
                let p = PathBuf::from(fp);
                if p.is_absolute() { p } else { session_cwd.join(p) }
            }
        })
        .unwrap_or_else(|| session_cwd.to_path_buf());

    // 4. Most-specific grant covering the target.
    if let Some(mode) = effective_mode_for_path(&session_perms.path_modes, &target_path) {
        if action_tier <= mode {
            return PermissionCheckResult::Allowed;
        }
    }

    // 5. Exceeds ceiling (or no grant) — needs upgrade prompt.
    let grant_path = grant_path_for_prompt(tool, &target_path, session_cwd);
    let path_str = display_path(&grant_path);
    let rule_arg = bash_command.or(file_path);
    let summary = rule_arg.unwrap_or(tool).to_string();

    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode: action_tier,
        path: path_str,
        tool_summary: format!("{} {}", tool, summary),
    })
}

/// Extract absolute (`/foo/bar`) and tilde-prefixed (`~`, `~/foo`) tokens from
/// a bash command. Used to gate Bash by the paths it actually touches, not
/// just the session cwd's tier. Best-effort: doesn't handle quoted paths with
/// spaces, embedded `--flag=/path` forms, or command substitution. Catches the
/// common `cmd /path` and `cmd ~/path` forms.
fn extract_command_paths(cmd: &str) -> Vec<String> {
    cmd.split_whitespace()
        .filter(|t| t.starts_with('/') || t.starts_with("~/") || *t == "~")
        // Strip trailing punctuation introduced by compound shell syntax
        // (`cmd1; cmd2`, `cmd1 && cmd2`, redirects).
        .map(|t| t.trim_end_matches(';').trim_end_matches('&').trim_end_matches('|').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Expand `~`, `~/...`, or absolute paths to a `PathBuf`.
fn expand_path_arg(p: &str) -> PathBuf {
    if p == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(p))
    } else if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    }
}

/// Compute the path to offer in the "Switch this folder to {mode}" option.
fn grant_path_for_prompt(tool: &str, target: &Path, session_cwd: &Path) -> PathBuf {
    if matches!(tool, "Grep" | "Glob" | "Task") {
        return target.to_path_buf();
    }
    if target.starts_with(session_cwd) {
        session_cwd.to_path_buf()
    } else if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().map(Path::to_path_buf).unwrap_or_else(|| target.to_path_buf())
    }
}

fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        let s = path.to_string_lossy();
        let h = home.to_string_lossy();
        if s.starts_with(h.as_ref()) {
            return format!("~{}", &s[h.len()..]);
        }
    }
    path.to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------
// Prompt construction & parsing
// ---------------------------------------------------------------------------

pub fn permission_target_summary(tool: &str, args: &serde_json::Value, cwd: &Path) -> String {
    match tool {
        "Write" | "Edit" => normalize_tool_path_arg(cwd, args)
            .unwrap_or_else(|| "<unknown file>".to_string()),
        "Bash" => args
            .get("cmd")
            .or_else(|| args.get("command"))
            .and_then(|v| v.as_str())
            .map(|cmd| {
                if cmd.len() > 80 {
                    format!("{}...", &cmd[..77])
                } else {
                    cmd.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown command>".to_string()),
        "Patch" => args
            .get("diff")
            .or_else(|| args.get("patch"))
            .and_then(|v| v.as_str())
            .and_then(|diff| {
                diff.lines().find_map(|line| {
                    line.strip_prefix("+++ b/")
                        .or_else(|| line.strip_prefix("+++ "))
                        .map(|s| s.to_string())
                })
            })
            .unwrap_or_else(|| "<patch>".to_string()),
        "WebFetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| truncate_for_prompt(url, 120))
            .unwrap_or_else(|| "<unknown URL>".to_string()),
        "WebSearch" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate_for_prompt(q, 120))
            .unwrap_or_else(|| "<unknown query>".to_string()),
        _ => skill_tool_summary(args).unwrap_or_else(|| tool.to_string()),
    }
}

fn skill_tool_summary(args: &serde_json::Value) -> Option<String> {
    for key in &["query", "content", "id", "endpoint", "path", "url"] {
        if let Some(v) = args.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(truncate_for_prompt(v, 120));
            }
        }
    }
    None
}

fn truncate_for_prompt(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max.saturating_sub(3)])
    } else {
        s.to_string()
    }
}

/// Build AskUser question for an ExceedsCeiling prompt.
pub fn build_exceeds_ceiling_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: format!("Switch this folder to {}", target_mode),
                description: Some(format!("Grants {} on {} and children", target_mode, path)),
                preview: None,
            },
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("One-time approval, no persistence".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this action".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Parse user response to an ExceedsCeiling prompt.
pub fn parse_exceeds_ceiling_answer(
    selected: &str,
    target_mode: &PermissionMode,
) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Switch this folder to {}", target_mode) {
        PermissionAction::AllowSession
    } else {
        PermissionAction::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_mode_ordering() {
        assert!(PermissionMode::Chat < PermissionMode::Read);
        assert!(PermissionMode::Read < PermissionMode::Edit);
        assert!(PermissionMode::Edit < PermissionMode::Admin);
    }

    #[test]
    fn test_permission_mode_serde() {
        let json = serde_json::to_string(&PermissionMode::Edit).unwrap();
        assert_eq!(json, "\"edit\"");
        let mode: PermissionMode = serde_json::from_str("\"admin\"").unwrap();
        assert_eq!(mode, PermissionMode::Admin);
    }

    #[test]
    fn test_session_permissions_serde_roundtrip() {
        let mut sp = SessionPermissions::default();
        sp.path_modes.push(PathMode {
            path: "~/workspace/linggen".to_string(),
            mode: PermissionMode::Edit,
        });
        let json = serde_json::to_string_pretty(&sp).unwrap();
        let loaded: SessionPermissions = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].mode, PermissionMode::Edit);
        assert!(loaded.interactive);
    }

    #[test]
    fn test_session_permissions_load_legacy_fields_ignored() {
        let tmp = std::env::temp_dir().join("linggen_perm_legacy_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let legacy = r#"{
            "path_modes": [{"path": "~/x", "mode": "edit"}],
            "locked": true,
            "policy": {"on_exceed": "ask", "on_ask_rule": "ask"},
            "allows": ["Bash(git push *)"],
            "denied_sigs": ["Bash:rm -rf dist"]
        }"#;
        fs::write(tmp.join("permission.json"), legacy).unwrap();
        let loaded = SessionPermissions::load(&tmp);
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].path, "~/x");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_set_path_mode_prunes_children() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("/tmp/project/src/a.rs", PermissionMode::Edit);
        sp.set_path_mode("/tmp/project/src/b.rs", PermissionMode::Edit);
        sp.set_path_mode("/tmp/other", PermissionMode::Read);
        assert_eq!(sp.path_modes.len(), 3);

        sp.set_path_mode("/tmp/project", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 2);
        assert!(sp.path_modes.iter().any(|pm| pm.path == "/tmp/project"));
        assert!(sp.path_modes.iter().any(|pm| pm.path == "/tmp/other"));
    }

    #[test]
    fn test_set_path_mode_prunes_tilde_children() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace/linggen/src/main.rs", PermissionMode::Edit);
        sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 1);
        assert_eq!(sp.path_modes[0].path, "~/workspace/linggen");
    }

    #[test]
    fn test_classify_bash_read() {
        assert_eq!(classify_bash_command("ls"), BashClass::Read);
        assert_eq!(classify_bash_command("git status"), BashClass::Read);
        assert_eq!(classify_bash_command("cargo check"), BashClass::Read);
        assert_eq!(classify_bash_command("python --version"), BashClass::Read);
    }

    #[test]
    fn test_classify_bash_write() {
        assert_eq!(classify_bash_command("mkdir -p src/new"), BashClass::Write);
        assert_eq!(classify_bash_command("git push origin main"), BashClass::Write);
        assert_eq!(classify_bash_command("cargo build"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_admin() {
        assert_eq!(classify_bash_command("rm -rf dist"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
    }

    #[test]
    fn test_classify_bash_sysdoctor_read() {
        // sys-doctor's command vocabulary should classify as read so the skill
        // can run with mode: read instead of mode: admin on /.
        assert_eq!(classify_bash_command("sw_vers"), BashClass::Read);
        assert_eq!(classify_bash_command("vm_stat"), BashClass::Read);
        assert_eq!(classify_bash_command("uptime"), BashClass::Read);
        assert_eq!(classify_bash_command("netstat -an"), BashClass::Read);
        assert_eq!(classify_bash_command("ifconfig en0"), BashClass::Read);
        assert_eq!(classify_bash_command("sysctl -n hw.ncpu"), BashClass::Read);
        assert_eq!(classify_bash_command("spctl --status"), BashClass::Read);
        assert_eq!(classify_bash_command("csrutil status"), BashClass::Read);
        assert_eq!(classify_bash_command("fdesetup status"), BashClass::Read);
        assert_eq!(classify_bash_command("brew list"), BashClass::Read);
        assert_eq!(classify_bash_command("brew --version"), BashClass::Read);
        assert_eq!(classify_bash_command("docker images"), BashClass::Read);
        assert_eq!(classify_bash_command("docker system df"), BashClass::Read);
        assert_eq!(classify_bash_command("docker ps"), BashClass::Read);
        // Mutations stay admin.
        assert_eq!(classify_bash_command("brew install foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker rm container"), BashClass::Admin);
    }

    #[test]
    fn test_classify_bash_compound() {
        assert_eq!(classify_bash_command("ls && rm foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("ls | grep foo"), BashClass::Read);
        assert_eq!(classify_bash_command("mkdir d && cp a b"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_redirect() {
        assert_eq!(classify_bash_command("echo hi > out.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("ls > files.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("du -sh ~/Desktop 2>/dev/null"), BashClass::Read);
    }

    #[test]
    fn test_hardcoded_deny_sudo() {
        assert!(is_hardcoded_deny("sudo apt install foo"));
        assert!(is_hardcoded_deny("sudo -i"));
        assert!(is_hardcoded_deny("sudoedit /etc/hosts"));
        assert!(is_hardcoded_deny("ls && sudo rm /etc/foo"));
    }

    #[test]
    fn test_hardcoded_deny_rm_rf_root() {
        assert!(is_hardcoded_deny("rm -rf /"));
        assert!(is_hardcoded_deny("rm -rf /*"));
        assert!(is_hardcoded_deny("rm -fr /"));
        assert!(!is_hardcoded_deny("rm -rf dist"));
        assert!(!is_hardcoded_deny("rm -rf ./build"));
    }

    #[test]
    fn test_hardcoded_deny_dd_blockdev() {
        assert!(is_hardcoded_deny("dd if=/dev/zero of=/dev/sda"));
        assert!(is_hardcoded_deny("dd of=/dev/disk2 if=foo.iso"));
        assert!(is_hardcoded_deny("dd if=foo of=/dev/nvme0n1"));
        assert!(!is_hardcoded_deny("dd if=/dev/zero of=/tmp/blob bs=1M count=10"));
    }

    #[test]
    fn test_hardcoded_deny_mkfs() {
        assert!(is_hardcoded_deny("mkfs /dev/sda1"));
        assert!(is_hardcoded_deny("mkfs.ext4 /dev/sda1"));
        assert!(is_hardcoded_deny("mkfs.btrfs /dev/sdb"));
    }

    #[test]
    fn test_hardcoded_deny_forkbomb() {
        assert!(is_hardcoded_deny(":(){:|:&};:"));
        assert!(is_hardcoded_deny(":() { :|:& };:"));
    }

    #[test]
    fn test_hardcoded_deny_chown_chmod_root() {
        assert!(is_hardcoded_deny("chown -R nobody /"));
        assert!(is_hardcoded_deny("chmod -R 777 /"));
        assert!(!is_hardcoded_deny("chown user:user /etc/hosts"));
    }

    #[test]
    fn test_hardcoded_deny_safe_commands() {
        assert!(!is_hardcoded_deny("ls"));
        assert!(!is_hardcoded_deny("cargo build"));
        assert!(!is_hardcoded_deny("git push origin main"));
        assert!(!is_hardcoded_deny(""));
    }

    #[test]
    fn test_effective_mode_for_path_basic() {
        let modes = vec![
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Edit },
            PathMode { path: "~/workspace/other".to_string(), mode: PermissionMode::Read },
        ];

        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/linggen/src/main.rs")),
                Some(PermissionMode::Edit),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/other/README.md")),
                Some(PermissionMode::Read),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("Documents/notes.txt")),
                None,
            );
        }
    }

    #[test]
    fn test_effective_mode_no_zone_default() {
        // Critical: /tmp no longer auto-grants edit.
        let modes: Vec<PathMode> = vec![];
        assert_eq!(effective_mode_for_path(&modes, Path::new("/tmp/x")), None);
        assert_eq!(effective_mode_for_path(&modes, Path::new("/var/tmp/x")), None);
    }

    #[test]
    fn test_effective_mode_most_specific_wins() {
        let modes = vec![
            PathMode { path: "~/workspace".to_string(), mode: PermissionMode::Read },
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Admin },
        ];
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/linggen/x")),
                Some(PermissionMode::Admin),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/other/y")),
                Some(PermissionMode::Read),
            );
        }
    }

    #[test]
    fn test_chat_mode_returns_needs_prompt_regardless_of_interactive() {
        // Contract: `check_permission` is mode-driven and does NOT consult
        // `session_permissions.interactive`. Chat mode always returns
        // NeedsPrompt(ExceedsCeiling) for any concrete tool. The interactive
        // distinction lives in tool_exec.rs:
        //   - interactive=true  (owner)            → prompt fires, user opts in
        //   - interactive=false (mission/consumer) → silent block, no prompt
        // This separation lets the permission classifier stay simple while
        // the dispatcher chooses the right user-facing behavior per session.
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace", PermissionMode::Chat);

        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace");
            let target = home.join("workspace/file.txt");
            let target_s = target.to_str().unwrap();

            // Owner: interactive = true.
            sp.interactive = true;
            let r1 = check_permission("Read", None, Some(target_s), &cwd, &sp, None);
            assert!(
                matches!(r1, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "owner case: expected NeedsPrompt, got {r1:?}",
            );

            // Mission / consumer: interactive = false. Same classifier output —
            // tool_exec is what turns this into a silent block.
            sp.interactive = false;
            let r2 = check_permission("Read", None, Some(target_s), &cwd, &sp, None);
            assert!(
                matches!(r2, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "non-interactive case: expected NeedsPrompt, got {r2:?}",
            );
        }
    }

    #[test]
    fn test_session_permissions_default_is_interactive() {
        // Owners default to interactive. Mission scheduler and SessionPolicy
        // flip this to false explicitly for unattended runs.
        let sp = SessionPermissions::default();
        assert!(sp.interactive);
    }

    #[test]
    fn test_check_permission_within_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);

            let result = check_permission(
                "Read", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));

            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));
        }
    }

    #[test]
    fn test_check_permission_exceeds_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Read);

            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
            );
            assert!(matches!(
                result,
                PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
            ));
        }
    }

    #[test]
    fn test_check_permission_no_grant_outside_cwd_prompts() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);

            // Read /etc/hosts — outside any grant, must prompt.
            let result = check_permission(
                "Read", None, Some("/etc/hosts"),
                &cwd, &sp, None,
            );
            assert!(matches!(
                result,
                PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
            ));
        }
    }

    #[test]
    fn test_check_permission_bash_args_gate_by_arg_path_not_cwd() {
        // The headline bug this gate fixes: a session with read on /A and cwd
        // /A could previously run `bash ls /B` because the gate only checked
        // cwd's tier. Now the path arg is checked — if /B isn't covered, the
        // call prompts to upgrade /B.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("a");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);

            // Read /B — /B has no grant — should prompt for /B's parent.
            let result = check_permission(
                "Bash", Some("ls /tmp/foo"), None,
                &cwd, &sp, None,
            );
            assert!(
                matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "ls /tmp/foo from cwd {} should prompt — /tmp/foo not covered. got {result:?}",
                cwd.display(),
            );

            // Now grant read on /tmp — should pass.
            sp.set_path_mode("/tmp", PermissionMode::Read);
            let result = check_permission(
                "Bash", Some("ls /tmp/foo"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "ls /tmp/foo with read on /tmp should be allowed. got {result:?}");
        }
    }

    #[test]
    fn test_check_permission_bash_no_args_uses_cwd() {
        // Bash without absolute path args (e.g. `cargo build`, `ls .`) operates
        // in cwd — the cwd's tier is what gates it.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("project");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Edit);

            // `cargo build` is write-class (edit tier), no path args → cwd check.
            let result = check_permission(
                "Bash", Some("cargo build"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "cargo build in edit-mode cwd should be allowed. got {result:?}");
        }
    }

    #[test]
    fn test_check_permission_bash_arg_already_granted_passes() {
        // Skill activation grants admin on ~/.linggen/skills. A bash command
        // that only references that granted path (e.g. `bash ~/.linggen/skills/foo/run.sh`)
        // should pass without re-prompting, even if cwd has a lower tier.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("project");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);
            sp.set_path_mode("~/.linggen/skills", PermissionMode::Admin);

            let result = check_permission(
                "Bash", Some("bash ~/.linggen/skills/foo/run.sh"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "bash on a granted path should be allowed even if cwd is lower-tier. got {result:?}");
        }
    }

    #[test]
    fn test_extract_command_paths() {
        assert_eq!(extract_command_paths("ls"), Vec::<String>::new());
        assert_eq!(extract_command_paths("ls /tmp"), vec!["/tmp"]);
        assert_eq!(extract_command_paths("cat /etc/hosts"), vec!["/etc/hosts"]);
        assert_eq!(extract_command_paths("ls /a /b"), vec!["/a", "/b"]);
        assert_eq!(extract_command_paths("ls ~/Desktop"), vec!["~/Desktop"]);
        // Compound — both parts.
        assert_eq!(extract_command_paths("ls /a; ls /b"), vec!["/a", "/b"]);
        assert_eq!(extract_command_paths("ls /a && ls /b"), vec!["/a", "/b"]);
        // Flag with embedded path (false negative — acceptable trade-off).
        assert_eq!(extract_command_paths("cargo install --root=/usr/local"),
                   Vec::<String>::new());
        // Relative paths aren't extracted — fall back to cwd check.
        assert_eq!(extract_command_paths("cat ./local.txt"), Vec::<String>::new());
    }

    #[test]
    fn test_check_permission_hardcoded_deny_overrides_admin() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~", PermissionMode::Admin);
        let cwd = dirs::home_dir().unwrap_or_default();

        let result = check_permission(
            "Bash", Some("sudo rm -rf /"), None,
            &cwd, &sp, None,
        );
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_tool_action_tier() {
        assert_eq!(tool_action_tier("Read"), PermissionMode::Read);
        assert_eq!(tool_action_tier("WebFetch"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Write"), PermissionMode::Edit);
        assert_eq!(tool_action_tier("Bash"), PermissionMode::Admin);
    }

    #[test]
    fn test_capability_tool_tier_comes_from_registry() {
        assert_eq!(tool_action_tier("Memory_query"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Memory_write"), PermissionMode::Edit);
    }

    #[test]
    fn test_parse_skill_tier() {
        assert_eq!(parse_skill_tier("read"), Some(PermissionMode::Read));
        assert_eq!(parse_skill_tier("edit"), Some(PermissionMode::Edit));
        assert_eq!(parse_skill_tier("admin"), Some(PermissionMode::Admin));
        assert_eq!(parse_skill_tier("nonsense"), None);
    }

    #[test]
    fn test_build_exceeds_ceiling_question() {
        let q = build_exceeds_ceiling_question("Edit src/main.rs", &PermissionMode::Edit, "~/work");
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Switch this folder to edit");
        assert_eq!(q.options[1].label, "Allow once");
        assert_eq!(q.options[2].label, "Deny");
    }

    #[test]
    fn test_parse_exceeds_ceiling_answer() {
        let mode = PermissionMode::Edit;
        assert_eq!(
            parse_exceeds_ceiling_answer("Switch this folder to edit", &mode),
            PermissionAction::AllowSession,
        );
        assert_eq!(
            parse_exceeds_ceiling_answer("Allow once", &mode),
            PermissionAction::AllowOnce,
        );
        assert_eq!(parse_exceeds_ceiling_answer("Deny", &mode), PermissionAction::Deny);
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        assert_eq!(permission_target_summary("Bash", &args, Path::new("/tmp")), "cargo build");
    }
}
