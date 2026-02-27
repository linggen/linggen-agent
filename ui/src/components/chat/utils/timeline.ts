import type { AgentRunInfo, AgentRunContextMessage } from '../../../types';
import type { TimelineEvent, ToolIntent } from '../types';

export const formatRunLabel = (run: AgentRunInfo) => {
  const ts = Number(run.started_at || 0);
  const time = ts > 0 ? new Date(ts * 1000).toLocaleTimeString() : '-';
  const shortId = run.run_id.length > 10 ? run.run_id.slice(0, 10) : run.run_id;
  return `${run.status} • ${time} • ${shortId}`;
};

export const formatTs = (ts?: number) => {
  if (!ts || ts <= 0) return '-';
  return new Date(ts * 1000).toLocaleTimeString();
};

export const previewValue = (value: string, maxChars = 100) =>
  value.length <= maxChars ? value : `${value.slice(0, maxChars)}... (${value.length} chars)`;

export const parseToolIntent = (content: string): ToolIntent | null => {
  const trimmed = content.trim();
  if (!trimmed) return null;
  if (/^Calling tool:/i.test(trimmed)) {
    const name = trimmed.replace(/^Calling tool:\s*/i, '').trim();
    return { name: name || 'unknown' };
  }
  if (/^Running command:/i.test(trimmed)) {
    const cmd = trimmed.replace(/^Running command:\s*/i, '').trim();
    return { name: 'Bash', detail: cmd || undefined };
  }
  if (/^Delegating to subagent:/i.test(trimmed)) {
    const target = trimmed.replace(/^Delegating to subagent:\s*/i, '').trim();
    return { name: 'Task', detail: target ? `target=${target}` : undefined };
  }
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== 'object') return null;
    const type = typeof parsed.type === 'string' ? parsed.type : '';
    if (type === 'tool') {
      const tool = typeof parsed.tool === 'string' ? parsed.tool : 'tool';
      if (tool === 'Bash' || tool === 'bash') {
        const cmd = typeof parsed.args?.cmd === 'string' ? parsed.args.cmd.trim() : '';
        return { name: tool, detail: cmd ? previewValue(cmd) : undefined };
      }
      if (tool === 'Task' || tool === 'delegate_to_agent') {
        const target = typeof parsed.args?.target_agent_id === 'string'
          ? parsed.args.target_agent_id.trim()
          : '';
        return {
          name: tool,
          detail: target ? `target=${target}` : undefined,
        };
      }
      return { name: tool };
    }
    if (type && type !== 'finalize_task') {
      return { name: type };
    }
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

export const parseTaskEvent = (content: string): string | null => {
  const trimmed = content.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed?.type === 'finalize_task') return 'Finalized task';
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

export const buildRunTimeline = (
  run?: AgentRunInfo,
  messages: AgentRunContextMessage[] = [],
  children: AgentRunInfo[] = []
): TimelineEvent[] => {
  const events: TimelineEvent[] = [];
  if (run) {
    events.push({
      ts: Number(run.started_at || 0),
      label: `Run started (${run.agent_id})`,
      kind: 'run',
    });
    if (run.ended_at) {
      events.push({
        ts: Number(run.ended_at || 0),
        label: `Run ended (${run.status})`,
        detail: run.detail || undefined,
        kind: 'run',
      });
    }
  }
  for (const child of children) {
    events.push({
      ts: Number(child.started_at || 0),
      label: `Spawned subagent: ${child.agent_id}`,
      kind: 'subagent',
    });
    if (child.ended_at) {
      events.push({
        ts: Number(child.ended_at || 0),
        label: `Subagent returned: ${child.agent_id} (${child.status})`,
        detail: child.detail || undefined,
        kind: 'subagent',
      });
    }
  }
  for (const msg of messages) {
    const tool = parseToolIntent(msg.content);
    if (tool) {
      events.push({
        ts: Number(msg.timestamp || 0),
        label: `Tool: ${tool.name}`,
        detail: [tool.detail, `${msg.from_id}${msg.to_id ? ` -> ${msg.to_id}` : ''}`]
          .filter(Boolean)
          .join(' • '),
        kind: 'tool',
      });
      continue;
    }
    const taskEvent = parseTaskEvent(msg.content);
    if (taskEvent) {
      events.push({
        ts: Number(msg.timestamp || 0),
        label: taskEvent,
        detail: msg.from_id,
        kind: 'task',
      });
    }
  }
  return events
    .filter((evt) => evt.ts > 0)
    .sort((a, b) => a.ts - b.ts)
    .slice(-40);
};
