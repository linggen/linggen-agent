const MAX_DIFF_BYTES: usize = 200 * 1024;

pub fn validate_unified_diff(diff: &str) -> Vec<String> {
    let mut errs = Vec::new();

    if diff.trim().is_empty() {
        errs.push("empty diff".to_string());
        return errs;
    }

    if diff.len() > MAX_DIFF_BYTES {
        errs.push(format!(
            "diff too large ({} bytes > {} bytes)",
            diff.len(),
            MAX_DIFF_BYTES
        ));
    }

    let mut has_header = false;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            has_header = true;
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let a = parts[0].trim_start_matches("a/");
                let b = parts[1].trim_start_matches("b/");
                if !path_is_safe(a) {
                    errs.push(format!("unsafe path in diff: {}", a));
                }
                if !path_is_safe(b) {
                    errs.push(format!("unsafe path in diff: {}", b));
                }
            }
        }
        if let Some(path) = line.strip_prefix("--- ") {
            if let Err(e) = validate_patch_path(path, "original") {
                errs.push(e);
            }
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            if let Err(e) = validate_patch_path(path, "modified") {
                errs.push(e);
            }
        }
    }

    if !has_header {
        errs.push("missing 'diff --git' header".to_string());
    }

    errs
}

fn path_is_safe(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') || path.starts_with("\\") {
        return false;
    }
    if path.contains("..") {
        return false;
    }
    true
}

fn validate_patch_path(raw: &str, label: &str) -> Result<(), String> {
    let path = raw.trim();
    if path == "/dev/null" {
        return Ok(());
    }
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    if !path_is_safe(path) {
        return Err(format!("unsafe {} path in diff: {}", label, raw));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diff_fails() {
        let errs = validate_unified_diff("");
        assert!(errs.iter().any(|e| e.contains("empty diff")));
    }

    #[test]
    fn missing_header_fails() {
        let errs = validate_unified_diff("--- a/x\n+++ b/x\n");
        assert!(errs.iter().any(|e| e.contains("missing 'diff --git'")));
    }

    #[test]
    fn unsafe_header_path_fails() {
        let diff = "diff --git a/../x b/x\n--- a/x\n+++ b/x\n";
        let errs = validate_unified_diff(diff);
        assert!(errs.iter().any(|e| e.contains("unsafe path")));
    }

    #[test]
    fn unsafe_patch_path_fails() {
        let diff = "diff --git a/x b/x\n--- /abs\n+++ b/x\n";
        let errs = validate_unified_diff(diff);
        assert!(errs.iter().any(|e| e.contains("unsafe original path")));
    }

    #[test]
    fn dev_null_is_allowed() {
        let diff = "diff --git a/x b/x\n--- /dev/null\n+++ b/x\n";
        let errs = validate_unified_diff(diff);
        assert!(errs.is_empty());
    }

    #[test]
    fn size_limit_is_enforced() {
        let mut diff = String::from("diff --git a/x b/x\n");
        diff.push_str(&"x".repeat(MAX_DIFF_BYTES + 1));
        let errs = validate_unified_diff(&diff);
        assert!(errs.iter().any(|e| e.contains("diff too large")));
    }
}
