/// Encode an absolute project path into a directory-safe name.
///
/// `/Users/foo/project` → `-Users-foo-project`
///
/// Same convention as Claude Code's `~/.claude/projects/` encoding.
pub fn encode_project_path(path: &str) -> String {
    path.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_project_path() {
        assert_eq!(encode_project_path("/Users/foo/project"), "-Users-foo-project");
        assert_eq!(encode_project_path("/tmp/p"), "-tmp-p");
        assert_eq!(encode_project_path("relative"), "relative");
    }
}
