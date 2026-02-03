use anyhow::Result;
use std::path::Path;
use std::process::Command;

// Very conservative verification allowlist for v1.
// We only support a small set of safe-ish commands and block shell metacharacters.
pub fn run_check(cmd: &str, cwd: &Path) -> Result<String> {
    if cmd.is_empty() {
        anyhow::bail!("missing command");
    }

    // Block common shell metacharacters to avoid chaining/redirects.
    let blocked = [';', '&', '|', '>', '<', '`'];
    if cmd.chars().any(|c| blocked.contains(&c)) {
        anyhow::bail!("unsupported characters in command");
    }

    let parts = split_args(cmd)?;
    if parts.is_empty() {
        anyhow::bail!("empty command");
    }

    // Allowlist only a few cargo checks for now.
    if parts[0] != "cargo" {
        anyhow::bail!("only 'cargo' commands are allowed in /check for v1");
    }

    // Allow: cargo test | check | clippy | fmt --check
    let parts_str: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
    let allowed = is_allowed(parts_str.as_slice());

    if !allowed {
        anyhow::bail!("command not allowlisted");
    }

    let mut c = Command::new("cargo");
    c.current_dir(cwd);
    for arg in parts.iter().skip(1) {
        c.arg(arg);
    }

    let out = c.output()?;
    let mut s = String::new();
    s.push_str(&format!("exit: {}\n", out.status.code().unwrap_or(-1)));
    s.push_str("--- stdout ---\n");
    s.push_str(&truncate(&String::from_utf8_lossy(&out.stdout), 64 * 1024));
    s.push_str("\n--- stderr ---\n");
    s.push_str(&truncate(&String::from_utf8_lossy(&out.stderr), 64 * 1024));
    Ok(s)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut out = s[..max].to_string();
    out.push_str("\n... (truncated)\n");
    out
}

fn split_args(cmd: &str) -> Result<Vec<String>> {
    // Minimal, shell-free splitting: supports quotes.
    // Uses a tiny parser to avoid pulling in heavier dependencies.
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in cmd.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
            }
            c => cur.push(c),
        }
    }

    if in_single || in_double {
        anyhow::bail!("unterminated quote");
    }

    if !cur.is_empty() {
        out.push(cur);
    }

    Ok(out)
}

fn is_allowed(parts: &[&str]) -> bool {
    matches!(
        parts,
        ["cargo", "test", ..]
            | ["cargo", "check", ..]
            | ["cargo", "clippy", ..]
            | ["cargo", "fmt", "--check", ..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_basic() {
        let parts = split_args("cargo test").unwrap();
        assert_eq!(parts, vec!["cargo", "test"]);
    }

    #[test]
    fn split_args_quotes() {
        let parts = split_args("cargo test \"foo bar\"").unwrap();
        assert_eq!(parts, vec!["cargo", "test", "foo bar"]);
    }

    #[test]
    fn split_args_unterminated() {
        let err = split_args("cargo test \"foo").unwrap_err();
        assert!(err.to_string().contains("unterminated quote"));
    }

    #[test]
    fn allowlist_accepts_known() {
        assert!(is_allowed(&["cargo", "test"]));
        assert!(is_allowed(&["cargo", "check", "-p", "crate"]));
        assert!(is_allowed(&["cargo", "clippy", "--all"]));
        assert!(is_allowed(&["cargo", "fmt", "--check"]));
    }

    #[test]
    fn allowlist_rejects_unknown() {
        assert!(!is_allowed(&["cargo", "fmt"]));
        assert!(!is_allowed(&["echo", "hi"]));
    }
}
