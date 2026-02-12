import React, { useCallback, useMemo, useState, useEffect, useRef } from 'react';
import { Activity, Folder, RefreshCw, Trash2 } from 'lucide-react';
import { AgentTree } from './components/AgentTree';
import { AgentsCard } from './components/AgentsCard';
import { ModelsCard } from './components/ModelsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/ChatPanel';
import { HeaderBar } from './components/HeaderBar';
import type {
  AgentInfo,
  AgentTreeItem,
  AgentRunInfo,
  AgentRunSummary,
  AgentWorkInfo,
  ChatMessage,
  LeadState,
  ModelInfo,
  OllamaPsResponse,
  ProjectInfo,
  QueuedChatItem,
  SessionInfo,
  SkillInfo,
  SubagentInfo,
} from './types';

const SELECTED_AGENT_STORAGE_KEY = 'linggen-agent:selected-agent';
const LIVE_MESSAGE_GRACE_MS = 10_000;

type ToolCallMeta = {
  tool: string;
  args?: any;
};

const parseToolCallFromParsedPayload = (parsed: any): ToolCallMeta | null => {
  if (!parsed || typeof parsed !== 'object') return null;
  if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') {
    return {
      tool: parsed.tool,
      args: parsed.args && typeof parsed.args === 'object' ? parsed.args : undefined,
    };
  }
  if (
    typeof parsed?.type === 'string' &&
    parsed.type !== 'finalize_task' &&
    parsed.args &&
    typeof parsed.args === 'object'
  ) {
    return { tool: parsed.type, args: parsed.args };
  }
  return null;
};

const parseToolNameFromParsedPayload = (parsed: any): string | null =>
  parseToolCallFromParsedPayload(parsed)?.tool || null;

const parseToolNameFromMessage = (text: string): string | null => {
  try {
    const parsed = JSON.parse(text);
    return parseToolNameFromParsedPayload(parsed);
  } catch (_e) {
    // Non-JSON messages are ignored.
  }
  return null;
};

const parseToolCallFromMessage = (text: string): ToolCallMeta | null => {
  try {
    const parsed = JSON.parse(text);
    return parseToolCallFromParsedPayload(parsed);
  } catch (_e) {
    // Non-JSON messages are ignored.
  }
  return null;
};

const parseToolCallFromText = (text: string): ToolCallMeta | null => {
  const direct = parseToolCallFromMessage(text.trim());
  if (direct) return direct;
  const lines = text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const parsed = parseToolCallFromMessage(lines[i]);
    if (parsed) return parsed;
  }
  return null;
};

const TOOL_JSON_EMBEDDED_RE = /\{"type":"tool","tool":"([^"]+)","args":\{[\s\S]*?\}\}/g;
const TOOL_RESULT_LINE_RE = /^(Tool\s+[A-Za-z0-9_.:-]+\s*:|tool_error:|tool_not_allowed:)/i;

const statusLineForTool = (toolName?: string) => {
  const name = (toolName || '').trim().toLowerCase();
  if (name === 'read_file' || name === 'read') return 'Reading file...';
  if (name === 'write_file' || name === 'write') return 'Writing file...';
  if (name === 'run_command' || name === 'bash') return 'Running command...';
  if (name === 'search_rg' || name === 'grep' || name === 'smart_search' || name === 'find_file')
    return 'Searching...';
  if (name === 'list_files' || name === 'glob') return 'Listing files...';
  if (name === 'delegate_to_agent') return 'Delegating...';
  return name ? `Calling tool: ${name}` : 'Calling tool...';
};

const extractToolNamesFromText = (text: string): string[] => {
  const names: string[] = [];
  const direct = parseToolNameFromMessage(text.trim());
  if (direct) names.push(direct);
  const lines = text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  for (const line of lines) {
    const n = parseToolNameFromMessage(line);
    if (n) names.push(n);
  }
  let m: RegExpExecArray | null = null;
  TOOL_JSON_EMBEDDED_RE.lastIndex = 0;
  while ((m = TOOL_JSON_EMBEDDED_RE.exec(text)) !== null) {
    if (m[1]) names.push(m[1]);
  }
  return Array.from(new Set(names));
};

const extractToolNameFromRawText = (text: string): string | null => {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const lines = trimmed.split('\n').map((line) => line.trim()).filter(Boolean);
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const toolName = parseToolNameFromMessage(lines[i]);
    if (toolName) return toolName;
  }
  return parseToolNameFromMessage(trimmed);
};

const stripToolPayloadLines = (text: string): string => {
  const withoutEmbedded = text.replace(TOOL_JSON_EMBEDDED_RE, '').trim();
  const cleaned = withoutEmbedded
    .split('\n')
    .map((line) => line.trimEnd())
    .filter((line) => {
      const t = line.trim();
      if (!t) return false;
      if (parseToolNameFromMessage(t)) return false;
      if (TOOL_RESULT_LINE_RE.test(t)) return false;
      return true;
    })
    .join('\n')
    .trim();
  return cleaned;
};

const isToolResultMessage = (from?: string, text?: string) => {
  if (!text) return false;
  const trimmed = text.trim();
  if (!trimmed) return false;
  return TOOL_RESULT_LINE_RE.test(trimmed) || (from === 'system' && trimmed.startsWith('Tool '));
};

const isStatusLineText = (text: string) =>
  text === 'Thinking...' ||
  text === 'Model loading...' ||
  text === 'Reading file...' ||
  text === 'Writing file...' ||
  text === 'Running command...' ||
  text.startsWith('Running command:') ||
  text === 'Searching...' ||
  text === 'Listing files...' ||
  text === 'Delegating...' ||
  text.startsWith('Delegating to subagent:') ||
  text === 'Calling tool...' ||
  text.startsWith('Calling tool:');

const roleFromAgentId = (agentId: string): ChatMessage['role'] =>
  agentId === 'lead' ? 'lead' : agentId === 'coder' ? 'coder' : 'agent';

const previewText = (value: string, maxChars = 120) =>
  value.length <= maxChars ? value : `${value.slice(0, maxChars)}...`;

const firstStringArg = (args: any, keys: string[]) => {
  if (!args || typeof args !== 'object') return '';
  for (const key of keys) {
    const value = args[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
};

const formatCompletedToolLine = (call?: ToolCallMeta): string | null => {
  if (!call || !call.tool) return null;
  const tool = call.tool.trim().toLowerCase();
  const args = call.args;
  if (tool === 'read_file' || tool === 'read') {
    const path = firstStringArg(args, ['path', 'file', 'filepath']);
    return path ? `Read ${path}` : 'Read file';
  }
  if (tool === 'write_file' || tool === 'write') {
    const path = firstStringArg(args, ['path', 'file', 'filepath']);
    return path ? `Wrote ${path}` : 'Wrote file';
  }
  if (tool === 'search_rg' || tool === 'grep' || tool === 'smart_search' || tool === 'find_file') {
    const query = firstStringArg(args, ['query', 'pattern', 'q']);
    return query ? `Searched for ${previewText(query, 110)}` : 'Searched';
  }
  if (tool === 'list_files' || tool === 'glob') {
    const globs = Array.isArray(args?.globs)
      ? args.globs.filter((v: unknown) => typeof v === 'string').map((v: string) => v.trim()).filter(Boolean)
      : [];
    if (globs.length > 0) return `Listed files in ${previewText(globs.join(', '), 110)}`;
    return 'Listed files';
  }
  if (tool === 'run_command' || tool === 'bash') {
    const cmd = firstStringArg(args, ['cmd', 'command']);
    return cmd ? `Ran command: ${previewText(cmd, 110)}` : 'Ran command';
  }
  if (tool === 'delegate_to_agent') {
    const target = firstStringArg(args, ['target_agent_id']);
    return target ? `Delegated to ${target}` : 'Delegated to subagent';
  }
  return `Used ${tool}`;
};

const summarizeActivityEntries = (entries: string[]): string | undefined => {
  if (entries.length === 0) return undefined;
  const tools = entries
    .filter((line) => /^Calling tool:/i.test(line))
    .map((line) => line.replace(/^Calling tool:\s*/i, '').trim())
    .filter(Boolean);
  const uniqueTools = Array.from(new Set(tools));
  const phases = entries.filter((line) => !/^Calling tool:/i.test(line));
  const normalized = entries.map((line) => line.toLowerCase());
  const readCount = normalized.filter((v) => v.startsWith('read ') || v.includes('reading file') || v.includes('read_file')).length;
  const searchCount = normalized.filter((v) => v.startsWith('searched for ') || v.includes('searching') || v.includes('search_rg') || v.includes('grep') || v.includes('smart_search') || v.includes('find_file')).length;
  const listCount = normalized.filter((v) => v.startsWith('listed files') || v.includes('listing files') || v.includes('list_files') || v.includes('glob')).length;
  if (readCount > 0 || searchCount > 0 || listCount > 0) {
    const parts: string[] = [];
    if (readCount > 0) parts.push(`${readCount} file${readCount > 1 ? 's' : ''}`);
    if (searchCount > 0) parts.push(`${searchCount} search${searchCount > 1 ? 'es' : ''}`);
    if (listCount > 0) parts.push(`${listCount} list${listCount > 1 ? 's' : ''}`);
    return `Explored ${parts.join(', ')}`;
  }
  const phaseSummary =
    phases.length > 1 ? `${phases[0]} -> ${phases[phases.length - 1]}` : phases[0] || '';
  const toolSummary =
    tools.length > 0
      ? `${tools.length} tool call${tools.length > 1 ? 's' : ''}${
          uniqueTools.length > 0
            ? `: ${uniqueTools.slice(0, 3).join(', ')}${uniqueTools.length > 3 ? ', ...' : ''}`
            : ''
        }`
      : '';
  if (phaseSummary && toolSummary) return `${phaseSummary} • ${toolSummary}`;
  return toolSummary || phaseSummary;
};

const addActivityEntry = (msg: ChatMessage, entry: string): ChatMessage => {
  const clean = entry.trim();
  if (!clean) return msg;
  const entries = msg.activityEntries ? [...msg.activityEntries] : [];
  if (entries.length === 0 || entries[entries.length - 1] !== clean) {
    entries.push(clean);
  }
  return {
    ...msg,
    activityEntries: entries,
    activitySummary: summarizeActivityEntries(entries),
  };
};

const findLastGeneratingMessageIndex = (messages: ChatMessage[], agentId: string) => {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i];
    if (msg.from === agentId && msg.isGenerating) return i;
  }
  return -1;
};

const upsertGeneratingAgentMessage = (
  messages: ChatMessage[],
  agentId: string,
  text: string,
  activityLine?: string
) => {
  const idx = findLastGeneratingMessageIndex(messages, agentId);
  const now = new Date();
  if (idx >= 0) {
    const next = [...messages];
    let updated: ChatMessage = {
      ...next[idx],
      role: roleFromAgentId(agentId),
      from: agentId,
      to: next[idx].to || 'user',
      text,
      timestamp: now.toLocaleTimeString(),
      timestampMs: now.getTime(),
      isGenerating: true,
    };
    if (activityLine) {
      updated = addActivityEntry(updated, activityLine);
    }
    next[idx] = updated;
    return next;
  }
  let created: ChatMessage = {
    role: roleFromAgentId(agentId),
    from: agentId,
    to: 'user',
    text,
    timestamp: now.toLocaleTimeString(),
    timestampMs: now.getTime(),
    isGenerating: true,
  };
  if (activityLine) {
    created = addActivityEntry(created, activityLine);
  }
  return [
    ...messages,
    created,
  ];
};

const appendGeneratingActivity = (messages: ChatMessage[], agentId: string, activityLine: string) => {
  const idx = findLastGeneratingMessageIndex(messages, agentId);
  const now = new Date();
  if (idx >= 0) {
    const next = [...messages];
    next[idx] = addActivityEntry(
      {
        ...next[idx],
        timestamp: now.toLocaleTimeString(),
        timestampMs: now.getTime(),
      },
      activityLine
    );
    return next;
  }
  return upsertGeneratingAgentMessage(messages, agentId, activityLine, activityLine);
};

const normalizeToolStatusDetail = (detail?: string) =>
  (detail || '')
    .trim()
    .replace(/^calling tool:\s*/i, '')
    .replace(/^calling\s+/i, '')
    .trim();

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const collectActivityEntries = (
  nodes: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[]
) => {
  if (!nodes) return;
  Object.values(nodes).forEach((item) => {
    if (item.type === 'file') {
      if (!item.path || !item.agent) return;
      out.push({
        path: item.path,
        agent: item.agent,
        status: item.status || 'idle',
        lastModified: Number(item.last_modified || 0),
      });
      return;
    }
    collectActivityEntries(item.children, out);
  });
};

const splitFilePath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) {
    return { folder: '.', file: path };
  }
  return {
    folder: path.slice(0, idx) || '.',
    file: path.slice(idx + 1),
  };
};

const buildAgentWorkInfo = (tree: Record<string, AgentTreeItem>): Record<string, AgentWorkInfo> => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);

  const byAgent = entries.reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
    if (!acc[entry.agent]) acc[entry.agent] = [];
    acc[entry.agent].push(entry);
    return acc;
  }, {});

  const out: Record<string, AgentWorkInfo> = {};
  Object.entries(byAgent).forEach(([agent, list]) => {
    const active = list
      .filter((entry) => entry.status === 'working')
      .sort((a, b) => b.lastModified - a.lastModified);
    const current = active[0];
    if (!current) return;
    const parts = splitFilePath(current.path);
    out[agent] = {
      path: current.path,
      folder: parts.folder,
      file: parts.file,
      status: current.status,
      activeCount: active.length,
    };
  });

  return out;
};

const buildSubagentInfos = (
  tree: Record<string, AgentTreeItem>,
  mainAgentIds: string[],
  agentStatus: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>
): SubagentInfo[] => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);
  const mainSet = new Set(mainAgentIds.map((id) => id.toLowerCase()));

  const bySubagent = entries
    .filter((entry) => !mainSet.has(entry.agent.toLowerCase()))
    .reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
      if (!acc[entry.agent]) acc[entry.agent] = [];
      acc[entry.agent].push(entry);
      return acc;
    }, {});

  const out: SubagentInfo[] = Object.entries(bySubagent)
    .reduce<SubagentInfo[]>((acc, [id, list]) => {
      const sorted = list.slice().sort((a, b) => b.lastModified - a.lastModified);
      const active = sorted.filter((entry) => entry.status === 'working');
      const current = active[0] || sorted[0];
      if (!current) return acc;

      const parts = splitFilePath(current.path);
      const uniquePaths = Array.from(new Set(sorted.map((entry) => entry.path))).slice(0, 8);
      const liveStatus = agentStatus[id];

      acc.push({
        id,
        status: liveStatus || (active.length > 0 ? 'working' : 'idle'),
        path: current.path,
        file: parts.file,
        folder: parts.folder,
        activeCount: active.length,
        paths: uniquePaths,
      });

      return acc;
    }, [])
    .sort((a, b) => {
      const score = (status: string) => (status === 'working' ? 2 : status === 'thinking' ? 1 : 0);
      return score(b.status) - score(a.status) || a.id.localeCompare(b.id);
    });

  return out;
};

const App: React.FC = () => {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [selectedProjectRoot, setSelectedProjectRoot] = useState<string>('');
  const [agentTree, setAgentTree] = useState<Record<string, AgentTreeItem>>({});
  const [agentTreesByProject, setAgentTreesByProject] = useState<Record<string, Record<string, AgentTreeItem>>>({});
  const [newProjectPath, setNewProjectPath] = useState('');
  const [showAddProject, setShowAddProject] = useState(false);

  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);

  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  const [, setLogs] = useState<string[]>([]);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>(() => {
    if (typeof window === 'undefined') return 'lead';
    const stored = window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY);
    return stored || 'lead';
  });
  const [currentMode, setCurrentMode] = useState<'chat' | 'auto'>('auto');
  const [isRunning, _setIsRunning] = useState(false);
  const [agentStatus, setAgentStatus] = useState<Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>>({});
  const [agentStatusText, setAgentStatusText] = useState<Record<string, string>>({});
  const [queuedMessages, setQueuedMessages] = useState<QueuedChatItem[]>([]);
  const [agentRuns, setAgentRuns] = useState<AgentRunInfo[]>([]);
  const [cancellingRunIds, setCancellingRunIds] = useState<Record<string, boolean>>({});
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [, setFiles] = useState<unknown[]>([]);
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  
  const [leadState, setLeadState] = useState<LeadState | null>(null);
  
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastSseSeqRef = useRef(0);
  const lastAgentStatusRef = useRef<Record<string, string>>({});
  const pendingToolByAgentRef = useRef<Record<string, ToolCallMeta>>({});
  const mainAgents = useMemo(() => {
    const mains = agents.filter((agent) => (agent.kind || 'main') !== 'subagent');
    return mains.length > 0 ? mains : agents;
  }, [agents]);
  const mainAgentIds = useMemo(() => {
    const ids = mainAgents.map((agent) => agent.name.toLowerCase());
    return ids.length > 0 ? ids : ['lead', 'coder'];
  }, [mainAgents]);
  const agentWork = useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);
  const subagents = useMemo(
    () => buildSubagentInfos(agentTree, mainAgentIds, agentStatus),
    [agentTree, mainAgentIds, agentStatus]
  );
  const sortedAgentRuns = useMemo(() => {
    const statusScore = (status: string) => (status === 'running' ? 1 : 0);
    return [...agentRuns].sort(
      (a, b) => statusScore(b.status) - statusScore(a.status) || Number(b.started_at || 0) - Number(a.started_at || 0)
    );
  }, [agentRuns]);
  const mainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'main') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'subagent') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'main' || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningSubagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'subagent' || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const mainRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'main') continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.agent_kind !== 'subagent') continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const agentRunSummary = useMemo(() => {
    const out: Record<string, AgentRunSummary> = {};
    for (const agent of mainAgents) {
      const agentId = agent.name.toLowerCase();
      const latest = mainRunHistory[agentId]?.[0];
      if (!latest) continue;
      const children = sortedAgentRuns.filter((run) => run.parent_run_id === latest.run_id);
      const timelineEvents =
        1 +
        (latest.ended_at ? 1 : 0) +
        children.reduce((count, child) => count + 1 + (child.ended_at ? 1 : 0), 0);
      const lastEventAt = Math.max(
        Number(latest.ended_at || 0),
        Number(latest.started_at || 0),
        ...children.flatMap((child) => [Number(child.started_at || 0), Number(child.ended_at || 0)])
      );
      out[agentId] = {
        run_id: latest.run_id,
        status: latest.status,
        started_at: latest.started_at,
        ended_at: latest.ended_at,
        child_count: children.length,
        timeline_events: timelineEvents,
        last_event_at: lastEventAt,
      };
    }
    return out;
  }, [mainAgents, mainRunHistory, sortedAgentRuns]);

  const addLog = useCallback((msg: string) => {
    setLogs(prev => [...prev, `[${new Date().toLocaleTimeString()}] ${msg}`]);
  }, []);

  const [agentContext, setAgentContext] = useState<Record<string, { tokens: number; messages: number; tokenLimit?: number }>>({});

  const shouldHideInternalChatMessage = useCallback((from?: string, text?: string) => {
    if (!text) return false;
    const stripped = stripToolPayloadLines(text);
    const hasToolPayload = extractToolNamesFromText(text).length > 0;
    const isToolResult = isToolResultMessage(from, text);
    
    // Hide tool results and tool payloads that have no other text
    if ((isToolResult || hasToolPayload) && !stripped) return true;
    
    // Explicitly hide any message that looks like a raw read_file output if it's from 'system'
    if (from === 'system' && text.includes('read_file:')) return true;

    if (from !== 'system') return false;
    return text.startsWith('Starting autonomous loop for task:');
  }, []);

  const chatMessageKey = useCallback((msg: ChatMessage) => {
    const from = msg.from || msg.role;
    const to = msg.to || '';
    const ts = msg.timestampMs ?? 0;
    return `${from}|${to}|${ts}|${msg.text}`;
  }, []);

  const sameMessageContent = useCallback((a: ChatMessage, b: ChatMessage) => {
    const fromA = a.from || a.role;
    const fromB = b.from || b.role;
    const toA = a.to || '';
    const toB = b.to || '';
    return fromA === fromB && toA === toB && a.text === b.text;
  }, []);

  const isStructuredAgentMessage = useCallback((msg: ChatMessage) => {
    if ((msg.from || msg.role) === 'user') return false;
    try {
      const parsed = JSON.parse(msg.text);
      return typeof parsed?.type === 'string';
    } catch (_e) {
      return false;
    }
  }, []);

  const likelySameMessage = useCallback((a: ChatMessage, b: ChatMessage) => {
    if (!sameMessageContent(a, b)) return false;
    if (isStructuredAgentMessage(a) || isStructuredAgentMessage(b)) return true;
    const ta = a.timestampMs ?? 0;
    const tb = b.timestampMs ?? 0;
    if (ta === 0 || tb === 0) return true;
    return Math.abs(ta - tb) <= 120_000;
  }, [sameMessageContent, isStructuredAgentMessage]);

  const mergeChatMessages = useCallback((persisted: ChatMessage[], live: ChatMessage[]) => {
    if (persisted.length === 0) return live;
    if (live.length === 0) return persisted;

    const persistedWithActivity = persisted.map((msg) => {
      const matchingLive = live.find(
        (candidate) =>
          !candidate.isGenerating &&
          likelySameMessage(msg, candidate) &&
          !!candidate.activityEntries &&
          candidate.activityEntries.length > 0
      );
      if (!matchingLive) return msg;
      return {
        ...msg,
        activityEntries: matchingLive.activityEntries,
        activitySummary: matchingLive.activitySummary,
      };
    });

    const now = Date.now();
    const uniqueExtras = live.filter(
      (m) => {
        if (m.isGenerating) return true;
        if (persistedWithActivity.some((p) => likelySameMessage(p, m))) return false;
        if (m.role === 'user' || m.from === 'user') return true;
        const ts = m.timestampMs ?? now;
        // Keep non-persisted live messages briefly to bridge DB/state lag.
        return now - ts <= LIVE_MESSAGE_GRACE_MS;
      }
    );
    const merged = [...persistedWithActivity, ...uniqueExtras];
    const seen = new Set<string>();
    return merged.filter((m) => {
      const key = chatMessageKey(m);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [chatMessageKey, likelySameMessage]);

  useEffect(() => {
    if (chatMessages.length > lastChatCountRef.current) {
      chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
    }
    lastChatCountRef.current = chatMessages.length;
  }, [chatMessages.length]);

  const fetchProjects = useCallback(async () => {
    try {
      const resp = await fetch('/api/projects');
      const data = await resp.json();
      setProjects(data);
      setAgentTreesByProject((prev) => {
        const valid = new Set<string>(data.map((p: ProjectInfo) => p.path));
        const next: Record<string, Record<string, AgentTreeItem>> = {};
        Object.entries(prev).forEach(([path, tree]) => {
          if (valid.has(path)) next[path] = tree;
        });
        return next;
      });
      setSelectedProjectRoot((prev) => {
        if (data.length === 0) return '';
        if (prev && data.some((p: ProjectInfo) => p.path === prev)) return prev;
        return data[0].path;
      });
    } catch (e) {
      addLog(`Error fetching projects: ${e}`);
    }
  }, [addLog]);

  const addProject = async () => {
    if (!newProjectPath.trim()) return;
    try {
      await fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: newProjectPath }),
      });
      setNewProjectPath('');
      setShowAddProject(false);
      fetchProjects();
    } catch (e) {
      addLog(`Error adding project: ${e}`);
    }
  };

  const removeProject = async (path: string) => {
    if (!confirm(`Are you sure you want to remove project: ${path}?`)) return;
    try {
      await fetch('/api/projects', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path }),
      });
      fetchProjects();
    } catch (e) {
      addLog(`Error removing project: ${e}`);
    }
  };

  const pickFolder = async () => {
    try {
      const resp = await fetch('/api/utils/pick-folder');
      if (resp.ok) {
        const data = await resp.json();
        if (data.path) {
          setNewProjectPath(data.path);
        }
      } else {
        addLog("Folder picker not supported on this OS yet.");
      }
    } catch (e) {
      addLog(`Error picking folder: ${e}`);
    }
  };

  const fetchAgentTree = useCallback(async (projectRoot?: string) => {
    const root = projectRoot || selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/workspace/tree?project_root=${encodeURIComponent(root)}`);
      const data = await resp.json();
      setAgentTreesByProject((prev) => ({ ...prev, [root]: data }));
      if (root === selectedProjectRoot || !projectRoot) {
        setAgentTree(data);
      }
    } catch (e) {
      addLog(`Error fetching agent tree (${root}): ${e}`);
    }
  }, [selectedProjectRoot, addLog]);

  const fetchAllAgentTrees = useCallback(async () => {
    if (projects.length === 0) return;
    await Promise.all(projects.map((project) => fetchAgentTree(project.path)));
  }, [projects, fetchAgentTree]);

  const fetchFiles = useCallback(async (path = '') => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/files?project_root=${encodeURIComponent(selectedProjectRoot)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      setFiles(data);
      setCurrentPath(path);
    } catch (e) {
      addLog(`Error fetching files: ${e}`);
    }
  }, [selectedProjectRoot, addLog]);

  const readFile = async (path: string, projectRootOverride?: string) => {
    const root = projectRootOverride || selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(root)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      setSelectedFileContent(data.content);
      setSelectedFilePath(path);
    } catch (e) {
      addLog(`Error reading file: ${e}`);
    }
  };

  const selectAgentPathFromTree = (projectRoot: string, path: string) => {
    if (projectRoot !== selectedProjectRoot) {
      setSelectedProjectRoot(projectRoot);
    }
    readFile(path, projectRoot);
  };

  const closeFilePreview = () => {
    setSelectedFilePath(null);
    setSelectedFileContent(null);
  };

  const fetchLeadState = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const url = new URL('/api/lead/state', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      if (activeSessionId) url.searchParams.append('session_id', activeSessionId);
      
      const resp = await fetch(url.toString());
      const data = await resp.json();
      setLeadState(data);
      
      // Update chat messages from state if needed
      if (data.messages) {
        const msgs: ChatMessage[] = data.messages
          .filter(([meta, body]: any) => !shouldHideInternalChatMessage(meta.from, body))
          .flatMap(([meta, body]: any) => {
            const isUser = meta.from === 'user';
            const cleaned = isUser ? body : stripToolPayloadLines(String(body || ''));
            if (!isUser && !cleaned) return [];
            return [{
              role:
                meta.from === 'user'
                  ? 'user'
                  : meta.from === 'lead'
                    ? 'lead'
                    : meta.from === 'coder'
                      ? 'coder'
                      : 'agent',
              from: meta.from,
              to: meta.to,
              text: cleaned,
              timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
              timestampMs: Number(meta.ts || 0) * 1000,
            }];
          });
        setChatMessages(prev => mergeChatMessages(msgs, prev));
      }
    } catch (e) {
      addLog(`Error fetching Lead state: ${e}`);
    }
  }, [selectedProjectRoot, activeSessionId, shouldHideInternalChatMessage, mergeChatMessages, addLog]);

  const fetchSessions = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      const data = await resp.json();
      setSessions(data);
    } catch (e) {
      console.error('Failed to fetch sessions:', e);
    }
  }, [selectedProjectRoot]);

  const createSession = async () => {
    if (!selectedProjectRoot) return;
    const title = prompt("Enter session title:", "New Chat");
    if (!title) return;
    
    try {
      const resp = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, title }),
      });
      const data = await resp.json();
      setActiveSessionId(data.id);
      fetchSessions();
    } catch (e) {
      addLog(`Error creating session: ${e}`);
    }
  };

  const removeSession = async (id: string) => {
    if (!selectedProjectRoot || !confirm("Remove this session?")) return;
    try {
      await fetch('/api/sessions', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id }),
      });
      if (activeSessionId === id) setActiveSessionId(null);
      fetchSessions();
    } catch (e) {
      addLog(`Error removing session: ${e}`);
    }
  };

  const fetchSkills = useCallback(async () => {
    try {
      const resp = await fetch('/api/skills');
      const data = await resp.json();
      setSkills(data);
    } catch (e) {
      console.error('Failed to fetch skills:', e);
    }
  }, []);

  const fetchAgents = useCallback(async () => {
    try {
      const resp = await fetch('/api/agents');
      const data = await resp.json();
      setAgents(data);
    } catch (e) {
      console.error('Failed to fetch agents:', e);
    }
  }, []);

  const fetchModels = useCallback(async () => {
    try {
      const resp = await fetch('/api/models');
      const data = await resp.json();
      setModels(data);
    } catch (e) {
      console.error('Failed to fetch models:', e);
    }
  }, []);

  const fetchOllamaStatus = useCallback(async () => {
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) {
        const data = await resp.json();
        setOllamaStatus(data);
      }
    } catch (e) {
      console.error('Failed to fetch Ollama status:', e);
    }
  }, []);

  const fetchSettings = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/settings?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      if (!resp.ok) return;
      const data = await resp.json();
      setCurrentMode(data.mode === 'chat' ? 'chat' : 'auto');
    } catch (e) {
      console.error('Failed to fetch settings:', e);
    }
  }, [selectedProjectRoot]);

  const fetchAgentRuns = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const url = new URL('/api/agent-runs', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      url.searchParams.append('session_id', activeSessionId || 'default');
      const resp = await fetch(url.toString());
      if (!resp.ok) return;
      const data = await resp.json();
      setAgentRuns(Array.isArray(data) ? data : []);
    } catch (e) {
      addLog(`Error fetching agent runs: ${e}`);
    }
  }, [selectedProjectRoot, activeSessionId, addLog]);

  const cancelAgentRun = async (runId: string) => {
    if (!runId) return;
    setCancellingRunIds((prev) => ({ ...prev, [runId]: true }));
    try {
      await fetch('/api/agent-cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ run_id: runId }),
      });
      await Promise.all([fetchAgentRuns(), fetchLeadState(), fetchAllAgentTrees()]);
    } catch (e) {
      addLog(`Error cancelling run ${runId}: ${e}`);
    } finally {
      setCancellingRunIds((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
    }
  };

  const updateMode = async (mode: 'chat' | 'auto') => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch('/api/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, mode }),
      });
      if (!resp.ok) return;
      const data = await resp.json();
      setCurrentMode(data.mode === 'chat' ? 'chat' : 'auto');
    } catch (e) {
      console.error('Failed to update mode:', e);
    }
  };

  useEffect(() => {
    fetchProjects();
    fetchSkills();
    fetchAgents();
    fetchModels();
    
    const interval = setInterval(fetchOllamaStatus, 5000);
    fetchOllamaStatus();
    return () => clearInterval(interval);
  }, [fetchProjects, fetchSkills, fetchAgents, fetchModels, fetchOllamaStatus]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchFiles();
      fetchLeadState();
      fetchAgentTree(selectedProjectRoot);
      fetchAgentRuns();
      fetchSessions();
      fetchSettings();
      setAgentStatus({});
      setAgentStatusText({});
      setQueuedMessages([]);
    }
  }, [selectedProjectRoot, fetchFiles, fetchLeadState, fetchAgentTree, fetchAgentRuns, fetchSessions, fetchSettings]);

  useEffect(() => {
    if (projects.length === 0) return;
    fetchAllAgentTrees();
  }, [projects, fetchAllAgentTrees]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchLeadState();
      fetchAgentRuns();
      setQueuedMessages([]);
    }
  }, [activeSessionId, selectedProjectRoot, fetchLeadState, fetchAgentRuns]);

  useEffect(() => {
    if (!selectedProjectRoot) return;
    const interval = window.setInterval(() => {
      fetchLeadState();
      fetchAgentRuns();
    }, 2000);
    return () => window.clearInterval(interval);
  }, [selectedProjectRoot, activeSessionId, fetchLeadState, fetchAgentRuns]);

  useEffect(() => {
    window.localStorage.setItem(SELECTED_AGENT_STORAGE_KEY, selectedAgent);
  }, [selectedAgent]);

  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      setSelectedAgent(mainAgentIds[0]);
    }
  }, [mainAgentIds, selectedAgent]);

  useEffect(() => {
    const events = new EventSource('/api/events');
    events.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        if (typeof event.seq === 'number') {
          if (event.seq <= lastSseSeqRef.current) return;
          lastSseSeqRef.current = event.seq;
        }
        if (event.type === 'StateUpdated') {
          fetchLeadState();
          fetchFiles(currentPath);
          fetchAllAgentTrees();
          fetchAgentRuns();
        } else if (event.type === 'AgentStatus') {
          const lifecycle = typeof event.lifecycle === 'string' ? event.lifecycle.trim().toLowerCase() : '';
          const nextStatus: 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working' =
            event.status === 'calling_tool'
              ? 'calling_tool'
              : event.status === 'model_loading'
                ? 'model_loading'
              : event.status === 'thinking'
                ? 'thinking'
                : event.status === 'working'
                  ? 'working'
                  : 'idle';
          const prevStatus = lastAgentStatusRef.current[event.agent_id] || 'idle';
          if (lifecycle === 'done') {
            if (nextStatus === 'calling_tool') {
              const completed = formatCompletedToolLine(pendingToolByAgentRef.current[event.agent_id]);
              if (completed) {
                setChatMessages((prev) => appendGeneratingActivity(prev, event.agent_id, completed));
              }
              delete pendingToolByAgentRef.current[event.agent_id];
            }
            return;
          }

          if (nextStatus === 'calling_tool') {
            const tool = normalizeToolStatusDetail(event.detail);
            if (tool) {
              pendingToolByAgentRef.current[event.agent_id] = pendingToolByAgentRef.current[event.agent_id] || { tool };
            }
          } else if (!lifecycle && (nextStatus === 'thinking' || nextStatus === 'idle') && prevStatus === 'calling_tool') {
            const completed = formatCompletedToolLine(pendingToolByAgentRef.current[event.agent_id]);
            if (completed) {
              setChatMessages((prev) => appendGeneratingActivity(prev, event.agent_id, completed));
            }
            delete pendingToolByAgentRef.current[event.agent_id];
          }
          setAgentStatus((prev) => ({
            ...prev,
            [event.agent_id]: nextStatus,
          }));
          setAgentStatusText((prev) => ({
            ...prev,
            [event.agent_id]:
              typeof event.detail === 'string' && event.detail.trim().length > 0
                ? event.detail
                : nextStatus === 'calling_tool'
                  ? 'Calling Tool'
                  : nextStatus === 'model_loading'
                    ? 'Model Loading'
                  : nextStatus === 'thinking'
                    ? 'Thinking'
                    : nextStatus === 'working'
                      ? 'Working'
                      : 'Idle',
          }));
          if (nextStatus === 'model_loading' || nextStatus === 'thinking' || nextStatus === 'calling_tool') {
            const toolName = normalizeToolStatusDetail(event.detail);
            const placeholder =
              nextStatus === 'calling_tool'
                ? statusLineForTool(toolName || '')
                : nextStatus === 'model_loading'
                  ? 'Model loading...'
                  : 'Thinking...';
            setChatMessages((prev) =>
              upsertGeneratingAgentMessage(prev, event.agent_id, placeholder, placeholder)
            );
          } else if (nextStatus === 'idle') {
            setChatMessages((prev) => {
              const idx = findLastGeneratingMessageIndex(prev, event.agent_id);
              if (idx < 0 || !isStatusLineText(prev[idx].text)) return prev;
              const next = [...prev];
              next[idx] = { ...next[idx], isGenerating: false };
              return next;
            });
          }
          lastAgentStatusRef.current[event.agent_id] = nextStatus;
        } else if (event.type === 'ContextUsage') {
          setAgentContext((prev) => ({
            ...prev,
            [event.agent_id]: {
              tokens: Number(event.estimated_tokens || 0),
              messages: Number(event.message_count || 0),
              tokenLimit: typeof event.token_limit === 'number' ? Number(event.token_limit) : prev[event.agent_id]?.tokenLimit,
            },
          }));
        } else if (event.type === 'SettingsUpdated') {
          if (event.project_root === selectedProjectRoot) {
            setCurrentMode(event.mode === 'chat' ? 'chat' : 'auto');
          }
        } else if (event.type === 'QueueUpdated') {
          const session = activeSessionId || 'default';
          if (event.project_root === selectedProjectRoot && event.session_id === session) {
            setQueuedMessages(event.items || []);
          }
        } else if (event.type === 'Token') {
          setChatMessages(prev => {
            const idx = findLastGeneratingMessageIndex(prev, event.agent_id);
            const now = new Date();
            if (idx >= 0) {
              const next = [...prev];
              const currentText = next[idx].text;
              const streamed = isStatusLineText(currentText)
                ? event.token
                : `${currentText}${event.token}`;
              const rawToolName = extractToolNameFromRawText(streamed);
              const cleaned = stripToolPayloadLines(streamed);
              let nextMsg: ChatMessage = {
                ...next[idx],
                text: rawToolName ? (cleaned || currentText) : streamed,
                timestamp: now.toLocaleTimeString(),
                timestampMs: now.getTime(),
                isGenerating: true,
              };
              if (rawToolName) {
                nextMsg = addActivityEntry(nextMsg, `Calling tool: ${rawToolName}`);
              }
              next[idx] = {
                ...nextMsg,
              };
              return next;
            }

            const rawToolName = extractToolNameFromRawText(event.token);
            const clean = rawToolName ? stripToolPayloadLines(event.token) : event.token;
            return [
              ...prev,
              addActivityEntry(
                {
                  role: roleFromAgentId(event.agent_id),
                  from: event.agent_id,
                  to: 'user',
                  text: clean || 'Thinking...',
                  timestamp: now.toLocaleTimeString(),
                  timestampMs: now.getTime(),
                  isGenerating: true,
                },
                rawToolName ? `Calling tool: ${rawToolName}` : 'Thinking...'
              ),
            ];
          });
        } else if (event.type === 'Message') {
          const content = event.content || '';
          const toolNames = extractToolNamesFromText(content);
          if (toolNames.length > 0) {
            const toolCall = parseToolCallFromText(content);
            if (toolCall) {
              pendingToolByAgentRef.current[event.from] = toolCall;
            }
            setChatMessages((prev) => {
              let next = prev;
              for (const name of toolNames) {
                next = appendGeneratingActivity(next, event.from, statusLineForTool(name));
              }
              return next;
            });
            if (!stripToolPayloadLines(content)) {
              return;
            }
          }
          if (shouldHideInternalChatMessage(event.from, content)) {
            return;
          }
          const cleanedContent =
            event.from === 'user'
              ? content
              : stripToolPayloadLines(content);
          if (event.from !== 'user' && !cleanedContent) {
            return;
          }
          setChatMessages(prev => {
            const generatingIdx = findLastGeneratingMessageIndex(prev, event.from);
            if (generatingIdx >= 0) {
              const next = [...prev];
              next[generatingIdx] = {
                ...next[generatingIdx],
                text: cleanedContent,
                to: event.to || next[generatingIdx].to || 'user',
                isGenerating: false,
              };
              return next;
            }
            // If there's no streaming message to finalize, append as a new message.
            const role: ChatMessage['role'] =
              event.from === 'user'
                ? 'user'
                : event.from === 'lead'
                  ? 'lead'
                  : event.from === 'coder'
                    ? 'coder'
                    : 'agent';

            if (
              prev.some(
                (msg) =>
                  !msg.isGenerating &&
                  (msg.from || msg.role) === event.from &&
                  (msg.to || '') === (event.to || '') &&
                  msg.text === cleanedContent
              )
            ) {
              return prev;
            }

            return [...prev, {
              role,
              from: event.from,
              to: event.to,
              text: cleanedContent,
              timestamp: new Date().toLocaleTimeString(),
              timestampMs: Date.now(),
              isGenerating: false
            }];
          });
          fetchLeadState();
        } else if (event.type === 'Outcome') {
          fetchLeadState();
          fetchAgentRuns();
        }
      } catch (err) {
        // If we ever receive a malformed SSE payload (e.g. due to lag/drop),
        // fall back to a state refresh so tool actions still show up.
        console.error("SSE parse error", err);
        fetchLeadState();
        fetchAgentRuns();
      }
    };

    return () => events.close();
  }, [currentPath, selectedProjectRoot, activeSessionId, fetchLeadState, fetchFiles, fetchAllAgentTrees, fetchAgentRuns, shouldHideInternalChatMessage]);

  const sendChatMessage = async (userMessage: string, targetAgent?: string) => {
    if (!userMessage.trim() || !selectedProjectRoot) return;
    const agentToUse = targetAgent || selectedAgent;
    const now = new Date();

    setChatMessages(prev => [
      ...prev,
      {
        role: 'user',
        from: 'user',
        to: agentToUse,
        text: userMessage,
        timestamp: now.toLocaleTimeString(),
        timestampMs: now.getTime(),
        isGenerating: false,
      },
    ]);

    const trimmed = userMessage.trim().toLowerCase();
    if (trimmed === '/mode chat') {
      setCurrentMode('chat');
    } else if (trimmed === '/mode auto') {
      setCurrentMode('auto');
    }
    if (userMessage.startsWith('/user_story ')) {
      const story = userMessage.substring(12).trim();
      addLog(`Setting user story: ${story}`);
      await fetch('/api/task', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          project_root: selectedProjectRoot, 
          agent_id: 'lead', 
          task: story 
        }),
      });
      return;
    }

    try {
      const resp = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          project_root: selectedProjectRoot, 
          agent_id: agentToUse,
          message: userMessage,
          session_id: activeSessionId
        }),
      });
      const data = await resp.json();
      if (data?.status === 'queued') {
        return;
      }
      setAgentStatus((prev) => ({ ...prev, [agentToUse]: 'model_loading' }));
      setAgentStatusText((prev) => ({ ...prev, [agentToUse]: 'Model Loading' }));
      setChatMessages((prev) =>
        upsertGeneratingAgentMessage(prev, agentToUse, 'Model loading...', 'Model loading...')
      );
    } catch (e) {
      addLog(`Error in chat: ${e}`);
    }
  };

  const clearChat = async () => {
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/chat/clear', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          session_id: activeSessionId,
        }),
      });
      setChatMessages([]);
      setQueuedMessages([]);
      fetchLeadState();
    } catch (e) {
      addLog(`Error clearing chat: ${e}`);
    }
  };

  const [copyChatStatus, setCopyChatStatus] = useState<'idle' | 'copied' | 'error'>('idle');

  const copyChat = async () => {
    try {
      const headerLines = [
        `Linggen Agent Chat Export`,
        `Project: ${selectedProjectRoot || '(none)'}`,
        `Session: ${activeSessionId || 'default'}`,
        `Agent: ${selectedAgent}`,
        `ExportedAt: ${new Date().toISOString()}`,
        ``,
      ];

      const body = chatMessages
        .map((m) => {
          const from = m.from || m.role;
          const to = m.to ? ` → ${m.to}` : '';
          return `[${m.timestamp}] ${from}${to}\n${m.text}\n`;
        })
        .join('\n');

      const text = headerLines.join('\n') + body;
      await navigator.clipboard.writeText(text);
      setCopyChatStatus('copied');
      window.setTimeout(() => setCopyChatStatus('idle'), 1200);
    } catch (e) {
      console.error('Failed to copy chat', e);
      setCopyChatStatus('error');
      window.setTimeout(() => setCopyChatStatus('idle'), 1600);
    }
  };

  const refreshPageState = async () => {
    if (!selectedProjectRoot) return;
    fetchLeadState();
    fetchFiles(currentPath);
    fetchAllAgentTrees();
    fetchSessions();
    fetchSettings();
  };

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
      {/* Header */}
      <HeaderBar
        showAddProject={showAddProject}
        newProjectPath={newProjectPath}
        setNewProjectPath={setNewProjectPath}
        addProject={addProject}
        pickFolder={pickFolder}
        selectedAgent={selectedAgent}
        setSelectedAgent={setSelectedAgent}
        mainAgents={mainAgents}
        agentStatus={agentStatus}
        sessions={sessions}
        activeSessionId={activeSessionId}
        setActiveSessionId={setActiveSessionId}
        createSession={createSession}
        copyChat={copyChat}
        copyChatStatus={copyChatStatus}
        clearChat={clearChat}
        removeSession={removeSession}
        isRunning={isRunning}
        currentMode={currentMode}
        onModeChange={updateMode}
        agentContext={agentContext}
      />

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">
        
        {/* Left: Active Paths */}
        <aside className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f]">
          <div className="p-4 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
              <Activity size={14} /> Active Paths
            </h2>
            <div className="flex items-center gap-1">
              <button
                onClick={refreshPageState}
                disabled={!selectedProjectRoot || isRunning}
                className="p-1.5 hover:bg-blue-500/10 hover:text-blue-500 rounded-lg text-slate-500 transition-colors disabled:opacity-50"
                title="Refresh page state"
              >
                <RefreshCw size={14} className={isRunning ? 'animate-spin' : ''} />
              </button>
              <button
                onClick={() => setShowAddProject(!showAddProject)}
                className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-slate-500"
                title="Manage Projects"
              >
                <Folder size={14} />
              </button>
              {selectedProjectRoot && (
                <button
                  onClick={() => removeProject(selectedProjectRoot)}
                  className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-lg text-slate-500 transition-colors"
                  title="Remove Current Project"
                >
                  <Trash2 size={14} />
                </button>
              )}
            </div>
          </div>
          <AgentTree
            projects={projects}
            selectedProjectRoot={selectedProjectRoot}
            treesByProject={agentTreesByProject}
            onSelectProject={setSelectedProjectRoot}
            onSelectPath={selectAgentPathFromTree}
          />
        </aside>

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 p-2 min-h-0">
            <ChatPanel
              chatMessages={chatMessages}
              queuedMessages={queuedMessages}
              chatEndRef={chatEndRef}
              selectedAgent={selectedAgent}
              setSelectedAgent={setSelectedAgent}
              skills={skills}
              agents={agents}
              mainAgents={mainAgents}
              subagents={subagents}
              mainRunIds={mainRunIds}
              subagentRunIds={subagentRunIds}
              runningMainRunIds={runningMainRunIds}
              runningSubagentRunIds={runningSubagentRunIds}
              mainRunHistory={mainRunHistory}
              subagentRunHistory={subagentRunHistory}
              cancellingRunIds={cancellingRunIds}
              onCancelRun={cancelAgentRun}
              onSendMessage={sendChatMessage}
            />
          </div>
        </main>

        {/* Right: Status */}
        <aside className="w-80 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-4 gap-4 overflow-y-auto">
          <AgentsCard
            agents={mainAgents}
            leadState={leadState}
            isRunning={isRunning}
            selectedAgent={selectedAgent}
            agentStatus={agentStatus}
            agentStatusText={agentStatusText}
            agentWork={agentWork}
            agentRunSummary={agentRunSummary}
            agentContext={agentContext}
          />
          <ModelsCard models={models} ollamaStatus={ollamaStatus} chatMessages={chatMessages} />
        </aside>
      </div>

      <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={closeFilePreview} />

      <style>{`
        .custom-scrollbar { scrollbar-gutter: stable; }
        .custom-scrollbar::-webkit-scrollbar { width: 8px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
      `}</style>
    </div>
  );
};

export default App;
