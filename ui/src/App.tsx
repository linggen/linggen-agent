import React, { useCallback, useMemo, useState, useEffect, useRef } from 'react';
import { Activity, FilePenLine, Folder, RefreshCw, Trash2 } from 'lucide-react';
import { AgentTree } from './components/AgentTree';
import { AgentsCard } from './components/AgentsCard';
import { ModelsCard } from './components/ModelsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/ChatPanel';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { AgentSpecEditorModal } from './components/AgentSpecEditorModal';
import type {
  AgentInfo,
  AgentTreeItem,
  AgentRunInfo,
  AgentRunSummary,
  AgentWorkInfo,
  ChatMessage,
  WorkspaceState,
  ModelInfo,
  OllamaPsResponse,
  ProjectInfo,
  QueuedChatItem,
  SessionInfo,
  SkillInfo,
  SubagentInfo,
  UiSseMessage,
} from './types';

const SELECTED_AGENT_STORAGE_KEY = 'linggen-agent:selected-agent';
const LIVE_MESSAGE_GRACE_MS = 10_000;
const TOKEN_RATE_WINDOW_MS = 8_000;
const TOKEN_RATE_IDLE_RESET_MS = 10_000;

const parseToolNameFromParsedPayload = (parsed: any): string | null => {
  if (!parsed || typeof parsed !== 'object') return null;
  if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') {
    return parsed.tool;
  }
  if (
    typeof parsed?.type === 'string' &&
    parsed.type !== 'finalize_task' &&
    parsed.args &&
    typeof parsed.args === 'object'
  ) {
    return parsed.type;
  }
  return null;
};

const parseToolNameFromMessage = (text: string): string | null => {
  try {
    const parsed = JSON.parse(text);
    return parseToolNameFromParsedPayload(parsed);
  } catch (_e) {
    // Non-JSON messages are ignored.
  }
  return null;
};

const TOOL_JSON_EMBEDDED_RE = /\{"type":"tool","tool":"([^"]+)","args":\{[\s\S]*?\}\}/g;
const TOOL_RESULT_LINE_RE = /^(Tool\s+[A-Za-z0-9_.:-]+\s*:|tool_error:|tool_not_allowed:)/i;

const activityKind = (line?: string): string => {
  const t = String(line || '').trim().toLowerCase();
  if (!t) return '';
  if (t === 'reading file...' || t.startsWith('reading file:') || t.startsWith('read ')) return 'read';
  if (t === 'writing file...' || t.startsWith('writing file:') || t.startsWith('wrote ')) return 'write';
  if (t === 'running command...' || t.startsWith('running command:') || t.startsWith('ran command')) return 'bash';
  if (t === 'searching...' || t.startsWith('searching:') || t.startsWith('searched for ')) return 'grep';
  if (t === 'listing files...' || t.startsWith('listing files:') || t.startsWith('listed files')) return 'glob';
  if (t === 'delegating...' || t.startsWith('delegating to subagent:') || t.startsWith('delegated to ')) return 'delegate_to_agent';
  if (t === 'calling tool...' || t.startsWith('calling tool:')) return 'calling_tool';
  return '';
};

const isGenericActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t === 'reading file...' ||
    t === 'writing file...' ||
    t === 'running command...' ||
    t === 'searching...' ||
    t === 'listing files...' ||
    t === 'delegating...' ||
    t === 'calling tool...'
  );
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
  text.startsWith('Reading file:') ||
  text === 'Writing file...' ||
  text.startsWith('Writing file:') ||
  text === 'Running command...' ||
  text.startsWith('Running command:') ||
  text === 'Searching...' ||
  text.startsWith('Searching:') ||
  text === 'Listing files...' ||
  text.startsWith('Listing files:') ||
  text === 'Delegating...' ||
  text.startsWith('Delegating to subagent:') ||
  text === 'Calling tool...' ||
  text.startsWith('Calling tool:');

const roleFromAgentId = (agentId: string): ChatMessage['role'] =>
  agentId === 'user' ? 'user' : 'agent';

const MODEL_LOADING_ENTRY = 'Model loading...';

const orderActivityEntries = (entries: string[]) => {
  if (!entries.includes(MODEL_LOADING_ENTRY)) return entries;
  const rest = entries.filter((entry) => entry !== MODEL_LOADING_ENTRY);
  return [MODEL_LOADING_ENTRY, ...rest];
};

const normalizeMessageTextForDedup = (text: string) =>
  (text || '').replace(/\s+/g, ' ').trim();

const summarizeActivityEntries = (entries: string[], inProgress = false): string | undefined => {
  if (entries.length === 0) return undefined;
  const tools = entries
    .filter((line) => /^Calling tool:/i.test(line))
    .map((line) => line.replace(/^Calling tool:\s*/i, '').trim())
    .filter(Boolean);
  const uniqueTools = Array.from(new Set(tools));
  const phases = entries.filter((line) => !/^Calling tool:/i.test(line));
  const normalized = entries.map((line) => line.toLowerCase());
  const readCount = normalized.filter((v) => v.startsWith('read ') || v.includes('reading file')).length;
  const searchCount = normalized.filter((v) => v.startsWith('searched for ') || v.includes('searching') || v.includes('grep')).length;
  const listCount = normalized.filter((v) => v.startsWith('listed files') || v.includes('listing files') || v.includes('glob')).length;
  if (readCount > 0 || searchCount > 0 || listCount > 0) {
    const parts: string[] = [];
    if (readCount > 0) parts.push(`${readCount} file${readCount > 1 ? 's' : ''}`);
    if (searchCount > 0) parts.push(`${searchCount} search${searchCount > 1 ? 'es' : ''}`);
    if (listCount > 0) parts.push(`${listCount} list${listCount > 1 ? 's' : ''}`);
    return `${inProgress ? 'Exploring' : 'Explored'} ${parts.join(', ')}`;
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
  if (clean === MODEL_LOADING_ENTRY && entries.some((item) => item !== MODEL_LOADING_ENTRY)) {
    return msg;
  }
  if (entries.length === 0) {
    entries.push(clean);
  } else {
    const last = entries[entries.length - 1];
    if (last !== clean) {
      const lastKind = activityKind(last);
      const nextKind = activityKind(clean);
      if (
        lastKind &&
        lastKind === nextKind &&
        isGenericActivityLine(last) &&
        !isGenericActivityLine(clean)
      ) {
        entries[entries.length - 1] = clean;
      } else if (
        lastKind &&
        lastKind === nextKind &&
        !isGenericActivityLine(last) &&
        isGenericActivityLine(clean)
      ) {
        // Keep richer detail, drop regressive generic line.
      } else {
        entries.push(clean);
      }
    }
  }
  const ordered = orderActivityEntries(entries);
  return {
    ...msg,
    activityEntries: ordered,
    activitySummary: summarizeActivityEntries(ordered, Boolean(msg.isGenerating)),
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

const normalizeAgentStatus = (
  status?: string
): 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working' => {
  if (status === 'calling_tool') return 'calling_tool';
  if (status === 'model_loading') return 'model_loading';
  if (status === 'thinking') return 'thinking';
  if (status === 'working') return 'working';
  return 'idle';
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

type Page = 'main' | 'settings';

const App: React.FC = () => {
  const [currentPage, setCurrentPage] = useState<Page>('main');
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
    if (typeof window === 'undefined') return '';
    const stored = window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY);
    return stored || '';
  });
  const [currentMode, setCurrentMode] = useState<'chat' | 'auto'>('auto');
  const [agentStatus, setAgentStatus] = useState<Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>>({});
  // Derived: true when any agent is actively working
  const isRunning = Object.values(agentStatus).some((s) => s !== 'idle');
  const [agentStatusText, setAgentStatusText] = useState<Record<string, string>>({});
  const [queuedMessages, setQueuedMessages] = useState<QueuedChatItem[]>([]);
  const [agentRuns, setAgentRuns] = useState<AgentRunInfo[]>([]);
  const [cancellingRunIds, setCancellingRunIds] = useState<Record<string, boolean>>({});
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [showAgentSpecEditor, setShowAgentSpecEditor] = useState(false);
  
  const [workspaceState, setWorkspaceState] = useState<WorkspaceState | null>(null);
  
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastSseSeqRef = useRef(0);
  const tokenRateSamplesRef = useRef<Array<{ ts: number; tokens: number }>>([]);
  const lastTokenAtRef = useRef<number>(0);
  const lastAgentCharsRef = useRef<number>(0);
  const lastAgentCharsTsRef = useRef<number>(0);
  const hadGeneratingRef = useRef<boolean>(false);
  const [tokensPerSec, setTokensPerSec] = useState<number>(0);
  const mainAgents = useMemo(() => {
    const mains = agents.filter((agent) => (agent.kind || 'main') !== 'subagent');
    return mains.length > 0 ? mains : agents;
  }, [agents]);
  const mainAgentIds = useMemo(() => {
    const ids = mainAgents.map((agent) => agent.name.toLowerCase());
    return ids;
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

  const recomputeTokenRate = useCallback((nowMs?: number) => {
    const now = nowMs ?? Date.now();
    const cutoff = now - TOKEN_RATE_WINDOW_MS;
    const pruned = tokenRateSamplesRef.current.filter((sample) => sample.ts >= cutoff);
    tokenRateSamplesRef.current = pruned;
    if (pruned.length === 0) {
      setTokensPerSec(0);
      return;
    }
    const totalTokens = pruned.reduce((sum, sample) => sum + sample.tokens, 0);
    const oldestTs = pruned[0]?.ts ?? now;
    const elapsedSec = Math.max((now - oldestTs) / 1000, 0.25);
    const rate = totalTokens / elapsedSec;
    setTokensPerSec(Number.isFinite(rate) ? rate : 0);
  }, []);

  const [agentContext, setAgentContext] = useState<Record<string, { tokens: number; messages: number; tokenLimit?: number }>>({});

  const shouldHideInternalChatMessage = useCallback((from?: string, text?: string) => {
    if (!text) return false;
    const stripped = stripToolPayloadLines(text);
    const hasToolPayload = extractToolNamesFromText(text).length > 0;
    const isToolResult = isToolResultMessage(from, text);
    
    // Hide tool results and tool payloads that have no other text
    if ((isToolResult || hasToolPayload) && !stripped) return true;
    
    // Explicitly hide any message that looks like a raw Read output if it's from 'system'
    if (from === 'system' && text.includes('Read:')) return true;

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
    return (
      fromA === fromB &&
      toA === toB &&
      normalizeMessageTextForDedup(a.text) === normalizeMessageTextForDedup(b.text)
    );
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

  useEffect(() => {
    const now = Date.now();
    const agentChars = chatMessages.reduce((sum, msg) => {
      const from = msg.from || msg.role;
      if (from === 'user') return sum;
      return sum + String(msg.text || '').length;
    }, 0);
    const hasGeneratingAgent = chatMessages.some((msg) => {
      const from = msg.from || msg.role;
      return from !== 'user' && !!msg.isGenerating;
    });

    if (lastAgentCharsTsRef.current > 0) {
      const deltaChars = agentChars - lastAgentCharsRef.current;
      const elapsedMs = now - lastAgentCharsTsRef.current;
      const noRecentTokenEvents = now - lastTokenAtRef.current > 1_200;
      if (
        noRecentTokenEvents &&
        deltaChars > 0 &&
        elapsedMs > 0 &&
        (hasGeneratingAgent || hadGeneratingRef.current)
      ) {
        const tokens = Math.max(1, Math.floor((deltaChars + 3) / 4));
        tokenRateSamplesRef.current.push({ ts: now, tokens });
        lastTokenAtRef.current = now;
        recomputeTokenRate(now);
      }
    }

    lastAgentCharsRef.current = agentChars;
    lastAgentCharsTsRef.current = now;
    hadGeneratingRef.current = hasGeneratingAgent;
  }, [chatMessages, recomputeTokenRate]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (lastTokenAtRef.current === 0 || now - lastTokenAtRef.current > TOKEN_RATE_IDLE_RESET_MS) {
        tokenRateSamplesRef.current = [];
        setTokensPerSec(0);
        return;
      }
      recomputeTokenRate(now);
    }, 500);
    return () => window.clearInterval(timer);
  }, [recomputeTokenRate]);

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
      await resp.json();
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

  const fetchWorkspaceState = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const url = new URL('/api/workspace/state', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      if (activeSessionId) url.searchParams.append('session_id', activeSessionId);
      
      const resp = await fetch(url.toString());
      const data = await resp.json();
      setWorkspaceState(data);
      
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
      addLog(`Error fetching workspace state: ${e}`);
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

  const fetchAgents = useCallback(async (projectRootOverride?: string) => {
    const root = projectRootOverride || selectedProjectRoot;
    if (!root) {
      setAgents([]);
      return;
    }
    try {
      const resp = await fetch(`/api/agents?project_root=${encodeURIComponent(root)}`);
      const data = await resp.json();
      setAgents(data);
    } catch (e) {
      console.error('Failed to fetch agents:', e);
    }
  }, [selectedProjectRoot]);

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
      await Promise.all([fetchAgentRuns(), fetchWorkspaceState(), fetchAllAgentTrees()]);
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
      fetchWorkspaceState();
      fetchAgentTree(selectedProjectRoot);
      fetchAgentRuns();
      fetchSessions();
      fetchSettings();
      fetchAgents(selectedProjectRoot);
      setAgentStatus({});
      setAgentStatusText({});
      setQueuedMessages([]);
    }
  }, [selectedProjectRoot, fetchFiles, fetchWorkspaceState, fetchAgentTree, fetchAgentRuns, fetchSessions, fetchSettings, fetchAgents]);

  useEffect(() => {
    if (projects.length === 0) return;
    fetchAllAgentTrees();
  }, [projects, fetchAllAgentTrees]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchWorkspaceState();
      fetchAgentRuns();
      setQueuedMessages([]);
    }
  }, [activeSessionId, selectedProjectRoot, fetchWorkspaceState, fetchAgentRuns]);

  useEffect(() => {
    if (!selectedProjectRoot) return;
    const interval = window.setInterval(() => {
      fetchWorkspaceState();
      fetchAgentRuns();
    }, 2000);
    return () => window.clearInterval(interval);
  }, [selectedProjectRoot, activeSessionId, fetchWorkspaceState, fetchAgentRuns]);

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
        const item = JSON.parse(e.data) as UiSseMessage;
        if (typeof item.seq === 'number') {
          if (item.seq <= lastSseSeqRef.current) return;
          lastSseSeqRef.current = item.seq;
        }
        if (item.kind === 'run') {
          if (item.phase === 'sync' || item.phase === 'outcome') {
            fetchWorkspaceState();
            fetchFiles(currentPath);
            fetchAllAgentTrees();
            fetchAgentRuns();
          } else if (item.phase === 'context_usage' && item.data) {
            const agentIdKey =
              typeof item.data.agent_id === 'string'
                ? item.data.agent_id.toLowerCase()
                : (item.agent_id || '').toLowerCase();
            if (agentIdKey) {
              setAgentContext((prev) => ({
                ...prev,
                [agentIdKey]: {
                  tokens: Number(item.data.estimated_tokens || 0),
                  messages: Number(item.data.message_count || 0),
                  tokenLimit:
                    typeof item.data.token_limit === 'number'
                      ? Number(item.data.token_limit)
                      : prev[agentIdKey]?.tokenLimit,
                },
              }));
            }
          } else if (item.phase === 'settings_updated') {
            if (item.project_root === selectedProjectRoot) {
              const mode = String(item.data?.mode || '').toLowerCase();
              setCurrentMode(mode === 'chat' ? 'chat' : 'auto');
            }
          } else if (item.phase === 'change_report' && item.data) {
            fetchWorkspaceState();
            fetchFiles(currentPath);
          }
          return;
        }

        if (item.kind === 'queue') {
          const session = activeSessionId || 'default';
          if (item.project_root === selectedProjectRoot && item.session_id === session) {
            const items = Array.isArray(item.data?.items) ? item.data.items : [];
            setQueuedMessages(items);
          }
          return;
        }

        if (item.kind === 'activity') {
          const agentId = String(item.agent_id || '');
          if (!agentId) return;
          const statusRaw = String(item.data?.status || '').trim();
          const nextStatus = normalizeAgentStatus(statusRaw);
          const statusText = String(item.text || '').trim();

          if (statusRaw) {
            setAgentStatus((prev) => ({
              ...prev,
              [agentId]: nextStatus,
            }));
            setAgentStatusText((prev) => ({
              ...prev,
              [agentId]:
                statusText.length > 0
                  ? statusText
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
          }

          if (statusText.length > 0) {
            setChatMessages((prev) => appendGeneratingActivity(prev, agentId, statusText));
          } else if (nextStatus === 'model_loading' || nextStatus === 'thinking') {
            const placeholder = nextStatus === 'model_loading' ? 'Model loading...' : 'Thinking...';
            setChatMessages((prev) => upsertGeneratingAgentMessage(prev, agentId, placeholder));
          }

          if (nextStatus === 'idle' || item.phase === 'done' || item.phase === 'failed') {
            setChatMessages((prev) => {
              const idx = findLastGeneratingMessageIndex(prev, agentId);
              if (idx < 0) return prev;
              // Only finalize status-only messages here; real token content
              // gets finalized by the 'message' handler to avoid premature cutoff.
              const msgText = prev[idx].text || '';
              if (msgText && !isStatusLineText(msgText)) return prev;
              const next = [...prev];
              const entries = Array.isArray(next[idx].activityEntries) ? next[idx].activityEntries : [];
              next[idx] = {
                ...next[idx],
                isGenerating: false,
                activitySummary: summarizeActivityEntries(entries, false) || next[idx].activitySummary,
              };
              return next;
            });
          }
          return;
        }

        if (item.kind === 'token') {
          const agentId = String(item.agent_id || '');
          const isThinking = item.data?.thinking === true;
          if (!agentId) return;

          if (item.phase === 'done') {
            if (isThinking) {
              // Mark the current generating message as thinking (not final answer).
              setChatMessages((prev) => {
                const idx = findLastGeneratingMessageIndex(prev, agentId);
                if (idx >= 0) {
                  const next = [...prev];
                  next[idx] = { ...next[idx], isThinking: true };
                  return next;
                }
                return prev;
              });
            }
            return;
          }

          const tokenText = String(item.text || '');
          setChatMessages((prev) => {
            const idx = findLastGeneratingMessageIndex(prev, agentId);
            if (idx >= 0) {
              const next = [...prev];
              const isPlaceholder = isStatusLineText(next[idx].text || '');
              next[idx] = {
                ...next[idx],
                text: isPlaceholder ? tokenText : (next[idx].text || '') + tokenText,
                isGenerating: true,
                isThinking: isThinking || next[idx].isThinking,
                timestampMs: Date.now(),
              };
              return next;
            }
            return upsertGeneratingAgentMessage(prev, agentId, tokenText);
          });
          return;
        }

        if (item.kind === 'message') {
          const from = String(item.data?.from || item.agent_id || 'assistant');
          const to = String(item.data?.to || '');
          const content = String(item.text || '');
          if (!content) return;
          if (shouldHideInternalChatMessage(from, content)) return;

          if (from !== 'user' && isStatusLineText(content)) {
            setChatMessages((prev) => appendGeneratingActivity(prev, from, content));
            return;
          }

          const tsMs = Number(item.ts_ms || Date.now());
          setChatMessages((prev) => {
            const generatingIdx = findLastGeneratingMessageIndex(prev, from);
            if (generatingIdx >= 0) {
              const next = [...prev];
              const existingEntries = Array.isArray(next[generatingIdx].activityEntries)
                ? next[generatingIdx].activityEntries
                : [];
              next[generatingIdx] = {
                ...next[generatingIdx],
                text: content,
                to: to || next[generatingIdx].to || 'user',
                isGenerating: false,
                timestamp: new Date(tsMs).toLocaleTimeString(),
                timestampMs: tsMs,
                activitySummary:
                  summarizeActivityEntries(existingEntries, false) || next[generatingIdx].activitySummary,
              };
              return next;
            }

            if (
              prev.some(
                (msg) =>
                  !msg.isGenerating &&
                  (msg.from || msg.role) === from &&
                  (msg.to || '') === to &&
                  normalizeMessageTextForDedup(msg.text) ===
                    normalizeMessageTextForDedup(content)
              )
            ) {
              return prev;
            }

            return [
              ...prev,
              {
                role: from === 'user' ? 'user' : 'agent',
                from,
                to,
                text: content,
                timestamp: new Date(tsMs).toLocaleTimeString(),
                timestampMs: tsMs,
                isGenerating: false,
              },
            ];
          });
          fetchWorkspaceState();
        }
      } catch (err) {
        console.error("SSE parse error", err);
        fetchWorkspaceState();
        fetchAgentRuns();
      }
    };

    return () => events.close();
  }, [currentPath, selectedProjectRoot, activeSessionId, fetchWorkspaceState, fetchFiles, fetchAllAgentTrees, fetchAgentRuns, shouldHideInternalChatMessage]);

  const sendChatMessage = async (userMessage: string, targetAgent?: string) => {
    if (!userMessage.trim() || !selectedProjectRoot) return;
    const agentToUse = targetAgent || selectedAgent;
    if (!agentToUse) return;
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
          agent_id: agentToUse, 
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
      fetchWorkspaceState();
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
    fetchWorkspaceState();
    fetchFiles(currentPath);
    fetchAllAgentTrees();
    fetchSessions();
    fetchSettings();
    fetchAgents(selectedProjectRoot);
    fetchModels();
    fetchOllamaStatus();
  };

  return (
    <>
    {currentPage === 'settings' && (
      <SettingsPage
        onBack={() => {
          setCurrentPage('main');
          fetchModels();
          fetchOllamaStatus();
        }}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage === 'settings' ? ' hidden' : ''}`}>
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
        onOpenSettings={() => setCurrentPage('settings')}
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
          <div className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-3">
            <button
              onClick={() => setShowAgentSpecEditor(true)}
              disabled={!selectedProjectRoot}
              className="w-full inline-flex items-center justify-center gap-2 px-3 py-2 text-xs font-semibold rounded-lg border border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50"
            >
              <FilePenLine size={14} />
              Edit Agent Markdown
            </button>
          </div>
          <AgentsCard
            agents={mainAgents}
            workspaceState={workspaceState}
            isRunning={isRunning}
            selectedAgent={selectedAgent}
            agentStatus={agentStatus}
            agentStatusText={agentStatusText}
            agentWork={agentWork}
            agentRunSummary={agentRunSummary}
            agentContext={agentContext}
          />
          <ModelsCard
            models={models}
            ollamaStatus={ollamaStatus}
            chatMessages={chatMessages}
            tokensPerSec={tokensPerSec}
          />
        </aside>
      </div>

      <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={closeFilePreview} />
      <AgentSpecEditorModal
        open={showAgentSpecEditor}
        projectRoot={selectedProjectRoot}
        onClose={() => setShowAgentSpecEditor(false)}
        onChanged={() => {
          fetchAgents(selectedProjectRoot);
          fetchWorkspaceState();
          fetchAllAgentTrees();
        }}
      />

      <style>{`
        .custom-scrollbar { scrollbar-gutter: stable; }
        .custom-scrollbar::-webkit-scrollbar { width: 8px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
      `}</style>
    </div>
    </>
  );
};

export default App;
