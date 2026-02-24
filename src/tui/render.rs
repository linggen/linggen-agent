/// Rendering logic: converts DisplayBlock variants into Vec<Line<'static>>.
///
/// Visual style follows Claude Code conventions:
/// - Tool steps: ⏺ bullet with ⎿ Done result lines
/// - Subagent tree: box-drawing characters (├─, └─, │)
/// - Agent messages: markdown-rendered with [agent] prefix
/// - Plan blocks: checkbox indicators [x] [~] [ ]

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::display::*;
use super::markdown;

/// Render a single DisplayBlock into terminal lines.
pub fn render_block(block: &DisplayBlock, _width: u16) -> Vec<Line<'static>> {
    match block {
        DisplayBlock::UserMessage { text, image_count } => render_user_message(text, *image_count),
        DisplayBlock::AgentMessage { agent_id, text } => render_agent_message(agent_id, text),
        DisplayBlock::SystemMessage { text } => render_system_message(text),
        DisplayBlock::ToolGroup {
            steps,
            collapsed,
            estimated_tokens,
            duration_secs,
            ..
        } => render_tool_group(steps, *collapsed, *estimated_tokens, *duration_secs),
        DisplayBlock::SubagentGroup { entries } => render_subagent_group(entries),
        DisplayBlock::PlanBlock {
            summary,
            items,
            status,
        } => render_plan_block(summary, items, status),
        DisplayBlock::ChangeReport {
            files,
            truncated_count,
        } => render_change_report(files, *truncated_count),
    }
}

fn render_user_message(text: &str, image_count: usize) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(Color::White)),
    ];
    if image_count > 0 {
        spans.push(Span::styled(
            format!(
                " [{} image{}]",
                image_count,
                plural(image_count)
            ),
            Style::default().fg(Color::Magenta),
        ));
    }
    vec![Line::from(spans)]
}

fn render_agent_message(agent_id: &str, text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let md_lines = markdown::markdown_to_lines(text);

    if md_lines.is_empty() {
        return lines;
    }

    // Claude Code-style: ● bullet prefix with agent name
    let bullet = Span::styled(
        "● ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );

    if let Some(first) = md_lines.first() {
        let mut first_spans = vec![bullet];
        first_spans.extend(first.spans.iter().cloned());
        // Append dim agent tag at end of first line
        first_spans.push(Span::styled(
            format!("  [{agent_id}]"),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(first_spans));
    }

    // Remaining lines get indentation to align with first line's content (after "● ")
    let indent = "  ";
    for md_line in md_lines.iter().skip(1) {
        let mut spans = vec![Span::raw(indent.to_string())];
        spans.extend(md_line.spans.iter().cloned());
        lines.push(Line::from(spans));
    }

    lines
}

fn render_system_message(text: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::Yellow),
    ))]
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Format a token count as a compact string (e.g. "30.2k").
fn format_compact_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}m", tokens as f64 / 1_000_000.0)
    } else if tokens >= 10_000 {
        format!("{}k", tokens / 1000)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{}", tokens)
    }
}

/// Render a tool group with ⏺ bullets and ⎿ result lines.
pub fn render_tool_group(
    steps: &[ToolStep],
    collapsed: bool,
    estimated_tokens: Option<usize>,
    duration_secs: Option<u64>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if collapsed {
        // Collapsed summary: "Done (3 tool uses · 30.2k tokens · 16s)"
        let tool_count = steps.len();
        let mut parts = vec![format!(
            "{} tool use{}",
            tool_count,
            plural(tool_count)
        )];
        if let Some(tokens) = estimated_tokens {
            parts.push(format!("{} tokens", format_compact_tokens(tokens)));
        }
        if let Some(secs) = duration_secs {
            parts.push(format!("{}s", secs));
        }
        let summary = format!("Done ({})", parts.join(" · "));
        lines.push(Line::from(vec![
            Span::styled("  ⎿  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                summary,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        return lines;
    }

    for step in steps {
        let bullet_color = match step.status {
            StepStatus::InProgress => Color::Yellow,
            StepStatus::Done => Color::Green,
            StepStatus::Failed => Color::Red,
        };

        let result_text = match step.status {
            StepStatus::InProgress => "Running...",
            StepStatus::Done => "Done",
            StepStatus::Failed => "Failed",
        };

        // Tool line: ⏺ ToolName(args)
        let args_display = if step.args_summary.is_empty() {
            String::new()
        } else {
            format!("({})", truncate_str(&step.args_summary, 80))
        };

        lines.push(Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(bullet_color)),
            Span::styled(
                step.tool_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(args_display, Style::default().fg(Color::White)),
        ]));

        // Result line: ⎿  Done
        lines.push(Line::from(vec![
            Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                result_text.to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    // Summary line
    if steps.len() > 1 {
        let summary = tool_group_summary(steps);
        lines.push(Line::from(Span::styled(
            format!("  {summary}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    lines
}

/// Render in-progress tool steps (active group, not yet finalized).
/// Shows only the last step with "+N more" count for compact display.
pub fn render_tool_group_active(steps: &[ToolStep]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if steps.is_empty() {
        return lines;
    }

    // Show only the last step
    let last = &steps[steps.len() - 1];
    let bullet_color = match last.status {
        StepStatus::InProgress => Color::Yellow,
        StepStatus::Done => Color::Green,
        StepStatus::Failed => Color::Red,
    };

    let args_display = if last.args_summary.is_empty() {
        String::new()
    } else {
        format!("({})", truncate_str(&last.args_summary, 80))
    };

    lines.push(Line::from(vec![
        Span::styled("  ⏺ ", Style::default().fg(bullet_color)),
        Span::styled(
            last.tool_name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(args_display, Style::default().fg(Color::White)),
    ]));

    // Show "+N more" if there are previous steps
    let prev_count = steps.len() - 1;
    if prev_count > 0 {
        lines.push(Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled(
                format!(
                    "+{} more tool use{}",
                    prev_count,
                    plural(prev_count)
                ),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    lines
}

/// Build stats string: "· 13 tool uses · 62.7k tokens"
fn subagent_stats_str(entry: &SubagentEntry) -> String {
    let mut parts = Vec::new();
    if entry.tool_count > 0 {
        parts.push(format!(
            "{} tool use{}",
            entry.tool_count,
            plural(entry.tool_count)
        ));
    }
    if let Some(tokens) = entry.estimated_tokens {
        if tokens > 0 {
            parts.push(format!("{} tokens", format_compact_tokens(tokens)));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

/// Render a finalized (done) subagent group — Claude Code style with stats.
fn render_subagent_group(entries: &[SubagentEntry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let total = entries.len();

    let total_tools: usize = entries.iter().map(|e| e.tool_count).sum();
    let total_tokens: usize = entries.iter().filter_map(|e| e.estimated_tokens).sum();

    // Header
    let mut header_spans = vec![
        Span::styled("  ● ", Style::default().fg(Color::Green)),
        Span::styled(
            format!(
                "{total} agent{} finished",
                plural(total)
            ),
            Style::default().fg(Color::Green),
        ),
    ];
    // Append aggregate stats
    let mut agg_parts = Vec::new();
    if total_tools > 0 {
        agg_parts.push(format!(
            "{total_tools} tool use{}",
            plural(total_tools)
        ));
    }
    if total_tokens > 0 {
        agg_parts.push(format!("{} tokens", format_compact_tokens(total_tokens)));
    }
    if !agg_parts.is_empty() {
        header_spans.push(Span::styled(
            format!(" ({})", agg_parts.join(" · ")),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(header_spans));

    // Tree entries
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == total - 1;
        let branch = if is_last { "└" } else { "├" };
        let task_preview = truncate_str(&entry.task, 50);
        let stats = subagent_stats_str(entry);

        lines.push(Line::from(vec![
            Span::styled(
                format!("    {branch} "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                task_preview.to_string(),
                Style::default().fg(Color::White),
            ),
            Span::styled(stats, Style::default().fg(Color::DarkGray)),
        ]));
    }

    lines
}

/// Render live (in-progress) subagent tree — shows running status + current activity.
pub(super) fn render_subagent_group_live(entries: &[&SubagentEntry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let total = entries.len();
    let running = entries.iter().filter(|e| e.status == SubagentStatus::Running).count();
    let all_done = running == 0;

    // Header
    let (dot_color, header_color, header_text) = if all_done {
        (
            Color::Green,
            Color::Green,
            format!(
                "{total} agent{} finished",
                plural(total)
            ),
        )
    } else {
        (
            Color::Yellow,
            Color::Yellow,
            format!(
                "Running {running} agent{}…",
                plural(running)
            ),
        )
    };

    lines.push(Line::from(vec![
        Span::styled("  ● ", Style::default().fg(dot_color)),
        Span::styled(header_text, Style::default().fg(header_color)),
    ]));

    // Tree entries
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == total - 1;
        let branch = if is_last { "└" } else { "├" };
        let continuation = if is_last { " " } else { "│" };
        let is_running = entry.status == SubagentStatus::Running;
        let entry_color = match entry.status {
            SubagentStatus::Running => Color::Yellow,
            SubagentStatus::Failed => Color::Red,
            SubagentStatus::Done => Color::Green,
        };
        let task_preview = truncate_str(&entry.task, 50);
        let stats = subagent_stats_str(entry);

        lines.push(Line::from(vec![
            Span::styled(
                format!("    {branch} "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("● ", Style::default().fg(entry_color)),
            Span::styled(
                task_preview.to_string(),
                Style::default().fg(Color::White),
            ),
            Span::styled(stats, Style::default().fg(Color::DarkGray)),
        ]));

        // Current activity sub-line (only when running)
        if is_running {
            if let Some(activity) = &entry.current_activity {
                let activity_preview = truncate_str(activity, 60);
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {continuation}  └ "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        activity_preview.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    lines
}

pub(super) fn render_plan_block(summary: &str, items: &[PlanDisplayItem], status: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Header with status indicator (Claude Code style)
    let (header_icon, header_color) = match status {
        "planned" => ("◇", Color::Cyan),
        "completed" => ("✓", Color::Green),
        _ => ("✻", Color::Yellow), // executing/approved
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{header_icon} "),
            Style::default().fg(header_color),
        ),
        Span::styled(
            summary.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Items with tree connectors (Claude Code style)
    let total = items.len();
    for (i, item) in items.iter().enumerate() {
        let is_last = i == total - 1;
        let branch = if is_last { "└ " } else { "├ " };

        let (icon, icon_color) = match item.status.as_str() {
            "done" => ("✓", Color::Green),
            "in_progress" => ("■", Color::Yellow),
            "skipped" => ("-", Color::DarkGray),
            _ => ("□", Color::White),
        };

        let text_style = match item.status.as_str() {
            "done" => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT),
            "in_progress" => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().fg(Color::White),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {branch}"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
            Span::styled(item.title.clone(), text_style),
        ]));
    }

    lines
}

/// Generate a summary string for a tool group, e.g. "Read 2 files, ran 1 command".
fn tool_group_summary(steps: &[ToolStep]) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for step in steps {
        *counts.entry(step.tool_name.as_str()).or_insert(0) += 1;
    }

    let mut parts = Vec::new();
    let order = ["Read", "Write", "Edit", "Bash", "Grep", "Glob", "Delegate"];
    for name in order {
        if let Some(&count) = counts.get(name) {
            let label = match name {
                "Read" => format!("read {} file{}", count, plural(count)),
                "Write" => format!("wrote {} file{}", count, plural(count)),
                "Edit" => format!("edited {} file{}", count, plural(count)),
                "Bash" => format!("ran {} command{}", count, plural(count)),
                "Grep" => format!(
                    "searched {} pattern{}",
                    count,
                    plural(count)
                ),
                "Glob" => format!(
                    "listed {} glob{}",
                    count,
                    plural(count)
                ),
                "Delegate" => format!(
                    "delegated {} task{}",
                    count,
                    plural(count)
                ),
                _ => format!("{} x{}", name, count),
            };
            parts.push(label);
            counts.remove(name);
        }
    }
    // Remaining tools not in the order list
    for (name, count) in &counts {
        parts.push(format!("{} x{}", name, count));
    }

    if parts.is_empty() {
        "No tool calls".to_string()
    } else {
        parts.join(", ")
    }
}

/// Render a compact sticky plan progress view for the bottom of scrollable content.
/// Uses Claude Code-style tree connectors and strikethrough for completed items.
pub fn render_plan_sticky(summary: &str, items: &[PlanDisplayItem]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let done = items.iter().filter(|i| i.status == "done").count();
    let in_progress = items.iter().filter(|i| i.status == "in_progress").count();
    let total = items.len();

    // Header with spinner and progress stats
    let stats = if in_progress > 0 {
        format!(" ({}/{} done, {} running)", done, total, in_progress)
    } else {
        format!(" ({}/{} done)", done, total)
    };
    lines.push(Line::from(vec![
        Span::styled("✻ ", Style::default().fg(Color::Yellow)),
        Span::styled(
            summary.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(stats, Style::default().fg(Color::DarkGray)),
    ]));

    // Items with tree connectors
    for (i, item) in items.iter().enumerate() {
        let is_last = i == total - 1;
        let branch = if is_last { "└ " } else { "├ " };

        let (icon, icon_color) = match item.status.as_str() {
            "done" => ("✓", Color::Green),
            "in_progress" => ("■", Color::Yellow),
            "skipped" => ("-", Color::DarkGray),
            _ => ("□", Color::White),
        };

        let text_style = match item.status.as_str() {
            "done" => Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::CROSSED_OUT),
            "in_progress" => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            _ => Style::default().fg(Color::White),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {branch}"),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
            Span::styled(item.title.clone(), text_style),
        ]));
    }

    lines
}

fn render_change_report(files: &[ChangedFile], truncated_count: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Header: "Changed files (N)" in blue, matching the web UI style
    let count_label = if truncated_count > 0 {
        format!("Changed files ({} +{} more)", files.len(), truncated_count)
    } else {
        format!("Changed files ({})", files.len())
    };
    lines.push(Line::from(Span::styled(
        count_label,
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    )));

    for file in files {
        let summary_color = if file.summary.contains("Added") {
            Color::Green
        } else if file.summary.contains("Deleted") {
            Color::Red
        } else {
            Color::Yellow
        };

        // Count +/- lines from diff text
        let (added, deleted) = diff_line_counts(&file.diff);
        let stat_str = match (added > 0, deleted > 0) {
            (true, true) => format!("  +{added} -{deleted}"),
            (true, false) => format!("  +{added}"),
            (false, true) => format!("  -{deleted}"),
            _ => String::new(),
        };

        let mut spans = vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{:<20}", truncate_str(&file.summary, 20)),
                Style::default().fg(summary_color),
            ),
            Span::styled(
                file.path.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !stat_str.is_empty() {
            spans.push(Span::styled(stat_str, Style::default().fg(Color::DarkGray)));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(Span::styled(
        "  Review these diffs in the UI and rollback any file you don't want.",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    lines
}

/// Count added (+) and deleted (-) lines in a unified diff string.
fn diff_line_counts(diff: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut deleted = 0usize;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deleted += 1;
        }
    }
    (added, deleted)
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
