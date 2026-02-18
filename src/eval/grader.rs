use anyhow::Result;
use std::path::Path;
use std::time::Duration;

pub struct GradeResult {
    pub passed: bool,
    pub exit_code: i32,
    pub output: String,
}

const GRADER_TIMEOUT_SECS: u64 = 30;

pub async fn run_grader(
    grade_script: &Path,
    workspace_dir: &Path,
    task_name: &str,
    timeout_secs: Option<u64>,
) -> Result<GradeResult> {
    if !grade_script.exists() {
        return Ok(GradeResult {
            passed: false,
            exit_code: -1,
            output: format!("Grade script not found: {}", grade_script.display()),
        });
    }

    let timeout = timeout_secs.unwrap_or(GRADER_TIMEOUT_SECS);
    let result = tokio::time::timeout(
        Duration::from_secs(timeout),
        tokio::process::Command::new("bash")
            .arg(grade_script)
            .current_dir(workspace_dir)
            .env("EVAL_WORKSPACE", workspace_dir)
            .env("EVAL_TASK_NAME", task_name)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{}\nstderr: {}", stdout, stderr)
            };

            Ok(GradeResult {
                passed: exit_code == 0,
                exit_code,
                output: combined.trim().to_string(),
            })
        }
        Ok(Err(e)) => Ok(GradeResult {
            passed: false,
            exit_code: -1,
            output: format!("Failed to execute grader: {}", e),
        }),
        Err(_) => Ok(GradeResult {
            passed: false,
            exit_code: -1,
            output: format!("Grader timed out after {}s", timeout),
        }),
    }
}
