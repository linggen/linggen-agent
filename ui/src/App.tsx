import React, { useCallback, useMemo, useState, useEffect, useRef } from 'react';
import { Bot, FilePenLine, Sparkles, Target, Zap } from 'lucide-react';
import { AgentsCard } from './components/AgentsCard';
import { SessionNav } from './components/SessionNav';
import { TaskListCard } from './components/TaskListCard';
import { ModelsCard } from './components/ModelsCard';
import { CollapsibleCard } from './components/CollapsibleCard';
import { SkillsCard } from './components/SkillsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/ChatPanel';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { MemoryPage } from './components/MemoryPage';
import { MissionPage } from './components/MissionPage';
import { MissionSidebarCard } from './components/MissionSidebarCard';
import { AgentSpecEditorModal } from './components/AgentSpecEditorModal';
import type {
  AgentInfo,
  AgentTreeItem,
  AgentRunInfo,
  AgentRunSummary,
  AgentWorkInfo,
  ChatMessage,
  SubagentTreeEntry,
  WorkspaceState,
  ModelInfo,
  OllamaPsResponse,
  ProjectInfo,
  QueuedChatItem,
  SessionInfo,
  SkillInfo,
  SubagentInfo,
  UiSseMessage,
  IdlePromptEvent,
  MissionInfo,
} from './types';

const SELECTED_AGENT_STORAGE_KEY = 'linggen-agent:selected-agent';
const VERBOSE_MODE_STORAGE_KEY = 'linggen-agent:verbose-mode';
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
  if (t === 'reading file...' || t.startsWith('reading file:') || t.startsWith('read file')) return 'read';
  if (t === 'writing file...' || t.startsWith('writing file:') || t.startsWith('wrote ')) return 'write';
  if (t === 'editing file...' || t.startsWith('editing file:') || t.startsWith('edited ')) return 'edit';
  if (t === 'running command...' || t.startsWith('running command:') || t.startsWith('ran command')) return 'bash';
  if (t === 'searching...' || t.startsWith('searching:') || t.startsWith('searched')) return 'grep';
  if (t === 'listing files...' || t.startsWith('listing files:') || t.startsWith('listed files')) return 'glob';
  if (t === 'delegating...' || t.startsWith('delegating to subagent:') || t.startsWith('delegated to ')) return 'delegate_to_agent';
  if (t === 'calling tool...' || t.startsWith('calling tool:') || t.startsWith('used tool')) return 'calling_tool';
  return '';
};

const isGenericActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t === 'reading file...' ||
    t === 'writing file...' ||
    t === 'editing file...' ||
    t === 'running command...' ||
    t === 'searching...' ||
    t === 'listing files...' ||
    t === 'delegating...' ||
    t === 'calling tool...'
  );
};

/** Detect "in-progress" activity lines (present continuous verb). */
const isDoingActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t.startsWith('reading file') ||
    t.startsWith('writing file') ||
    t.startsWith('editing file') ||
    t.startsWith('running command') ||
    t.startsWith('searching') ||
    t.startsWith('listing files') ||
    t.startsWith('delegating') ||
    t.startsWith('calling tool')
  );
};

/** Detect "completed" activity lines (past tense verb). */
const isDoneActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t.startsWith('read file') ||
    t.startsWith('wrote ') ||
    t.startsWith('edited ') ||
    t.startsWith('ran command') ||
    t.startsWith('searched:') ||
    t.startsWith('searched for ') ||
    t.startsWith('listed files') ||
    t.startsWith('delegated to ') ||
    t.startsWith('used tool')
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

/** Strip embedded structured JSON (plan, tool calls, finalize_task) from text,
 *  handling nested braces correctly via depth counting. */
const stripEmbeddedStructuredJson = (text: string): string => {
  // Markers for JSON objects we want to strip from visible text
  const MARKERS = ['"type":"plan"', '"type":"tool"', '"type":"finalize_task"', '"type": "plan"', '"type": "tool"'];
  let result = text;

  for (const marker of MARKERS) {
    let searchFrom = 0;
    while (true) {
      const markerIdx = result.indexOf(marker, searchFrom);
      if (markerIdx < 0) break;

      // Find the opening { before the marker
      let start = -1;
      for (let i = markerIdx - 1; i >= 0; i--) {
        if (result[i] === '{') { start = i; break; }
      }
      if (start < 0) { searchFrom = markerIdx + marker.length; continue; }

      // Find the matching closing } using brace counting
      let depth = 0;
      let end = -1;
      for (let i = start; i < result.length; i++) {
        if (result[i] === '{') depth++;
        else if (result[i] === '}') {
          depth--;
          if (depth === 0) { end = i + 1; break; }
        }
      }
      if (end < 0) break; // Unbalanced braces (partial JSON during streaming), stop

      result = result.slice(0, start) + result.slice(end);
    }
  }

  return result.trim();
};

const isStatusLineText = (text: string) =>
  text === 'Thinking...' ||
  text === 'Thinking' ||
  text === 'Model loading...' ||
  text === 'Running' ||
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

/** Transient status lines that should not appear as activity entries. */
const isTransientStatus = (line: string): boolean => {
  const t = line.trim().toLowerCase();
  return t === 'thinking' || t === 'thinking...' || t === 'model loading' || t === 'model loading...' || t === 'running';
};

/** Extract the detail/target portion after the verb (e.g. "foo.rs" from "Reading file: foo.rs"). */
const activityDetail = (line?: string): string => {
  const idx = String(line || '').indexOf(': ');
  return idx >= 0 ? String(line || '').slice(idx + 2).trim() : '';
};

const addActivityEntry = (msg: ChatMessage, entry: string): ChatMessage => {
  const clean = entry.trim();
  if (!clean) return msg;

  // Skip transient status lines — they are not tool calls.
  if (isTransientStatus(clean)) return msg;

  const entries = msg.activityEntries ? [...msg.activityEntries] : [];
  const nextKind = activityKind(clean);

  if (entries.length === 0) {
    entries.push(clean);
  } else {
    // If this is a done-form line, scan backwards and replace its doing counterpart in-place.
    if (nextKind && isDoneActivityLine(clean)) {
      let replaced = false;
      const nextDetail = activityDetail(clean);
      for (let i = entries.length - 1; i >= 0; i--) {
        if (activityKind(entries[i]) === nextKind && isDoingActivityLine(entries[i])) {
          // Match on detail too: "Read file: foo.rs" must replace "Reading file: foo.rs",
          // not "Reading file: bar.rs". If both have details, they must match.
          const entryDetail = activityDetail(entries[i]);
          if (!nextDetail || !entryDetail || nextDetail === entryDetail) {
            entries[i] = clean;
            replaced = true;
            break;
          }
        }
      }
      if (!replaced) {
        // No doing counterpart found — just append.
        entries.push(clean);
      }
    } else {
      const last = entries[entries.length - 1];
      if (last === clean) {
        // Exact duplicate — skip.
      } else {
        const lastKind = activityKind(last);
        if (lastKind && lastKind === nextKind && isGenericActivityLine(last) && !isGenericActivityLine(clean)) {
          // Generic "Reading file..." → specific "Reading file: main.rs"
          entries[entries.length - 1] = clean;
        } else if (lastKind && lastKind === nextKind && !isGenericActivityLine(last) && isGenericActivityLine(clean)) {
          // Keep richer detail, drop regressive generic line.
        } else {
          entries.push(clean);
        }
      }
    }
  }

  // Strip any transient entries that may have leaked in earlier.
  const filtered = entries.filter((e) => !isTransientStatus(e));
  return {
    ...msg,
    activityEntries: filtered,
    activitySummary: summarizeActivityEntries(filtered, Boolean(msg.isGenerating)),
    toolCount: filtered.length,
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

/** Update the subagentTree on a parent agent's generating message. */
const updateParentSubagentTree = (
  messages: ChatMessage[],
  parentId: string,
  subagentId: string,
  updater: (entry: SubagentTreeEntry) => SubagentTreeEntry,
): ChatMessage[] => {
  // Find the last generating message from the parent agent.
  let targetIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if ((m.from || '').toLowerCase() === parentId.toLowerCase() && m.isGenerating) {
      targetIdx = i;
      break;
    }
  }
  // Fallback: last agent message from parent.
  if (targetIdx < 0) {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if ((m.from || '').toLowerCase() === parentId.toLowerCase() && m.role === 'agent') {
        targetIdx = i;
        break;
      }
    }
  }
  if (targetIdx < 0) return messages;

  const next = [...messages];
  const msg = next[targetIdx];
  const tree = msg.subagentTree ? [...msg.subagentTree] : [];
  const entryIdx = tree.findIndex((e) => e.subagentId === subagentId);
  if (entryIdx >= 0) {
    tree[entryIdx] = updater(tree[entryIdx]);
  }
  next[targetIdx] = { ...msg, subagentTree: tree };
  return next;
};

type Page = 'main' | 'settings' | 'memory' | 'mission';

const App: React.FC = () => {
  const [currentPage, setCurrentPage] = useState<Page>('main');
  const [initialSettingsTab, setInitialSettingsTab] = useState<import('./types').ManagementTab | undefined>(undefined);
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
  const [sessionCountsByProject, setSessionCountsByProject] = useState<Record<string, number>>({});

  const [, setLogs] = useState<string[]>([]);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>(() => {
    if (typeof window === 'undefined') return '';
    const stored = window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY);
    return stored || '';
  });
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
  const [mission, setMission] = useState<MissionInfo | null>(null);
  const [missionDraft, setMissionDraft] = useState<string | null>(null);
  const [idlePromptEvents, setIdlePromptEvents] = useState<IdlePromptEvent[]>([]);
  const [pendingPlan, setPendingPlan] = useState<import('./types').Plan | null>(null);
  const [pendingPlanAgentId, setPendingPlanAgentId] = useState<string | null>(null);
  const [pendingAskUser, setPendingAskUser] = useState<import('./types').PendingAskUser | null>(null);
  const [activePlan, setActivePlan] = useState<import('./types').Plan | null>(null);
  const [verboseMode, setVerboseMode] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false;
    return window.localStorage.getItem(VERBOSE_MODE_STORAGE_KEY) === 'true';
  });
  const runStartTsRef = useRef<Record<string, number>>({});
  const latestContextTokensRef = useRef<Record<string, number>>({});

  // Subagent tree tracking refs
  const subagentParentMapRef = useRef<Record<string, string>>({});
  const subagentStatsRef = useRef<Record<string, { toolCount: number; contextTokens: number }>>({});

  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastSseSeqRef = useRef(0);
  const chatClearTsRef = useRef(0);
  const tokenRateSamplesRef = useRef<Array<{ ts: number; tokens: number }>>([]);
  const lastTokenAtRef = useRef<number>(0);
  const lastAgentCharsRef = useRef<number>(0);
  const lastAgentCharsTsRef = useRef<number>(0);
  const hadGeneratingRef = useRef<boolean>(false);
  const [tokensPerSec, setTokensPerSec] = useState<number>(0);
  const mainAgents = agents;
  const mainAgentIds = useMemo(() => {
    return agents.map((agent) => agent.name.toLowerCase());
  }, [agents]);
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
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningSubagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const mainRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const agentRunSummary = useMemo(() => {
    const out: Record<string, AgentRunSummary> = {};
    for (const agent of agents) {
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
  }, [agents, mainRunHistory, sortedAgentRuns]);

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

  const isPlanMessage = useCallback((msg: ChatMessage): boolean => {
    const text = (msg.text || '').trim();
    // Pure JSON plan message
    try {
      const parsed = JSON.parse(text);
      if (parsed?.type === 'plan' && !!parsed?.plan) return true;
    } catch { /* not pure JSON */ }
    // Plan JSON embedded within other text
    if (text.includes('"type":"plan"') && text.includes('"plan":{')) return true;
    return false;
  }, []);

  const likelySameMessage = useCallback((a: ChatMessage, b: ChatMessage) => {
    // Plan messages from the same agent are always the same (content changes on updates).
    if (isPlanMessage(a) && isPlanMessage(b)) {
      return (a.from || a.role) === (b.from || b.role);
    }
    if (!sameMessageContent(a, b)) return false;
    if (isStructuredAgentMessage(a) || isStructuredAgentMessage(b)) return true;
    const ta = a.timestampMs ?? 0;
    const tb = b.timestampMs ?? 0;
    if (ta === 0 || tb === 0) return true;
    return Math.abs(ta - tb) <= 120_000;
  }, [sameMessageContent, isStructuredAgentMessage, isPlanMessage]);

  const mergeChatMessages = useCallback((persisted: ChatMessage[], live: ChatMessage[]) => {
    if (persisted.length === 0) {
      // Only keep actively generating messages; discard stale history.
      const generating = live.filter((m) => m.isGenerating);
      return generating;
    }
    if (live.length === 0) return dedupPlanMessages(persisted);

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

    return dedupPlanMessages(merged);
  }, [chatMessageKey, likelySameMessage, isPlanMessage]);

  /** Keep only the last plan message per agent; dedup non-plan by key. */
  const dedupPlanMessages = useCallback((messages: ChatMessage[]) => {
    // Deduplicate plan messages: keep only the last plan per agent.
    const lastPlanIdx = new Map<string, number>();
    messages.forEach((m, idx) => {
      if (isPlanMessage(m)) {
        lastPlanIdx.set(m.from || m.role || '', idx);
      }
    });

    const seen = new Set<string>();
    return messages.filter((m, idx) => {
      if (isPlanMessage(m)) {
        const agent = m.from || m.role || '';
        return lastPlanIdx.get(agent) === idx;
      }
      const key = chatMessageKey(m);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [chatMessageKey, isPlanMessage]);

  /** Always-deduped + structured-JSON-stripped messages for rendering.
   *  Plan messages are moved to the bottom and kept as one per agent. */
  const displayMessages = useMemo(() => {
    const deduped = dedupPlanMessages(chatMessages);
    const plans: ChatMessage[] = [];
    const nonPlans: ChatMessage[] = [];
    for (const msg of deduped) {
      if (isPlanMessage(msg)) {
        plans.push(msg);
      } else {
        // Strip embedded structured JSON (plan, tool, finalize) from agent text
        if ((msg.from || msg.role) !== 'user') {
          let text = msg.text || '';
          // Strip complete structured JSON objects
          text = stripEmbeddedStructuredJson(text);
          // Strip trailing partial JSON fragments during streaming (e.g. '{ "type' at end)
          if (msg.isGenerating) {
            text = text.replace(/\{\s*"type\b[^}]*$/s, '').trim();
          }
          if (text !== msg.text && text) {
            nonPlans.push({ ...msg, text });
            continue;
          }
        }
        nonPlans.push(msg);
      }
    }
    // Plans go at the bottom, one per agent, in-place updated
    return [...nonPlans, ...plans];
  }, [chatMessages, dedupPlanMessages, isPlanMessage]);

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
      // Skip repopulation during clear-chat cooldown (5s) to let server process the clear
      const inClearCooldown = chatClearTsRef.current > 0 && Date.now() - chatClearTsRef.current < 5000;
      if (data.messages && !inClearCooldown) {
        const msgs: ChatMessage[] = data.messages
          .filter(([meta, body]: any) => !shouldHideInternalChatMessage(meta.from, body))
          .flatMap(([meta, body]: any) => {
            const isUser = meta.from === 'user';
            let bodyStr = String(body || '');

            // If this is a pure plan JSON message, keep it as-is for plan rendering
            try {
              const parsed = JSON.parse(bodyStr);
              if (parsed?.type === 'plan' && parsed?.plan) {
                return [{
                  role: 'agent' as const,
                  from: meta.from,
                  to: meta.to,
                  text: bodyStr,
                  timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
                  timestampMs: Number(meta.ts || 0) * 1000,
                }];
              }
            } catch { /* not pure JSON */ }

            // Strip embedded plan JSON from agent text to avoid showing duplicate plan blocks
            if (!isUser) {
              bodyStr = stripEmbeddedStructuredJson(bodyStr);
            }

            const cleaned = isUser ? bodyStr : stripToolPayloadLines(bodyStr);
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
      const data: SessionInfo[] = await resp.json();
      setSessions(data);
      // Auto-select the most recent session when none is active
      setActiveSessionId((prev) => {
        if (prev && data.some((s: SessionInfo) => s.id === prev)) return prev;
        if (data.length > 0) return data[0].id;
        return null;
      });
    } catch (e) {
      console.error('Failed to fetch sessions:', e);
    }
  }, [selectedProjectRoot]);

  const createSession = async () => {
    if (!selectedProjectRoot) return;
    const now = new Date();
    const title = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;

    try {
      const resp = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, title }),
      });
      const data = await resp.json();
      setActiveSessionId(data.id);
      fetchSessions();
      fetchAllSessionCounts();
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
      fetchAllSessionCounts();
    } catch (e) {
      addLog(`Error removing session: ${e}`);
    }
  };

  const renameSession = async (id: string, title: string) => {
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id, title }),
      });
      fetchSessions();
    } catch (e) {
      addLog(`Error renaming session: ${e}`);
    }
  };

  const fetchAllSessionCounts = useCallback(async () => {
    if (projects.length === 0) return;
    const counts: Record<string, number> = {};
    await Promise.all(
      projects.map(async (project) => {
        try {
          const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(project.path)}`);
          const data = await resp.json();
          counts[project.path] = Array.isArray(data) ? data.length : 0;
        } catch {
          counts[project.path] = 0;
        }
      }),
    );
    setSessionCountsByProject(counts);
  }, [projects]);

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

  const missionEndpointAvailable = useRef<boolean | null>(null);
  const prevMissionProjectRoot = useRef(selectedProjectRoot);

  // Reset endpoint availability when switching projects
  if (selectedProjectRoot !== prevMissionProjectRoot.current) {
    missionEndpointAvailable.current = null;
    prevMissionProjectRoot.current = selectedProjectRoot;
  }

  const fetchMission = useCallback(async () => {
    if (!selectedProjectRoot) return;
    if (missionEndpointAvailable.current === false) return;
    try {
      const url = new URL('/api/mission', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      const resp = await fetch(url.toString());
      if (resp.status === 404) {
        missionEndpointAvailable.current = false;
        return;
      }
      missionEndpointAvailable.current = true;
      if (!resp.ok) return;
      const data = await resp.json();
      setMission(data?.text ? data : null);
    } catch {
      // silently ignore — mission endpoint may not be available
    }
  }, [selectedProjectRoot]);

  const saveMission = async (text: string) => {
    if (!selectedProjectRoot || !text.trim()) return;
    try {
      await fetch('/api/mission', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, text: text.trim(), agents: [] }),
      });
      setMissionDraft(null);
      fetchMission();
    } catch (e) {
      addLog(`Error setting mission: ${e}`);
    }
  };

  const clearMission = async () => {
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/mission', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot }),
      });
      setMission(null);
    } catch (e) {
      addLog(`Error clearing mission: ${e}`);
    }
  };

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
      fetchAgents(selectedProjectRoot);
      fetchMission();
      setAgentStatus({});
      setAgentStatusText({});
      setQueuedMessages([]);
      setActivePlan(null);
    }
  }, [selectedProjectRoot, fetchFiles, fetchWorkspaceState, fetchAgentTree, fetchAgentRuns, fetchSessions, fetchAgents, fetchMission]);

  useEffect(() => {
    if (projects.length === 0) return;
    fetchAllAgentTrees();
    fetchAllSessionCounts();
  }, [projects, fetchAllAgentTrees, fetchAllSessionCounts]);

  useEffect(() => {
    if (selectedProjectRoot) {
      // Clear stale chat state before fetching the new session's data.
      setChatMessages([]);
      setQueuedMessages([]);
      setActivePlan(null);
      fetchWorkspaceState();
      fetchAgentRuns();
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
    window.localStorage.setItem(VERBOSE_MODE_STORAGE_KEY, String(verboseMode));
  }, [verboseMode]);

  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      // Default to 'ling' if available, otherwise first agent
      const preferred = mainAgentIds.includes('ling') ? 'ling' : mainAgentIds[0];
      setSelectedAgent(preferred);
    }
  }, [mainAgentIds, selectedAgent]);

  useEffect(() => {
    const events = new EventSource('/api/events');
    // Reset seq counter when (re)connecting — after server restart the seq resets to 0
    // but lastSseSeqRef still holds the old high value, causing all new events to be dropped.
    events.onopen = () => {
      lastSseSeqRef.current = 0;
    };
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
              const estTokens = Number(item.data.estimated_tokens || 0);
              latestContextTokensRef.current[agentIdKey] = estTokens;

              // If this agent is a known subagent, update its parent's tree
              const parentId = subagentParentMapRef.current[agentIdKey];
              if (parentId) {
                const stats = subagentStatsRef.current[agentIdKey];
                if (stats) stats.contextTokens = estTokens;
                setChatMessages((prev) =>
                  updateParentSubagentTree(prev, parentId, agentIdKey, (entry) => ({
                    ...entry,
                    contextTokens: estTokens,
                  }))
                );
              } else {
                setAgentContext((prev) => ({
                  ...prev,
                  [agentIdKey]: {
                    tokens: estTokens,
                    messages: Number(item.data.message_count || 0),
                    tokenLimit:
                      typeof item.data.token_limit === 'number'
                        ? Number(item.data.token_limit)
                        : prev[agentIdKey]?.tokenLimit,
                  },
                }));
              }
            }
          } else if (item.phase === 'plan_update' && item.data?.plan) {
            const plan = item.data.plan as import('./types').Plan;
            const agentId = String(item.agent_id || '');
            // Always update sidebar task list card.
            setActivePlan(plan);
            if (plan.status === 'planned' && plan.origin === 'user_requested') {
              setPendingPlan(plan);
              setPendingPlanAgentId(agentId);
            }
            // Update or insert plan message in chat (replace existing plan msg from same agent).
            const planText = JSON.stringify({ type: 'plan', plan });
            setChatMessages((prev) => {
              const existingIdx = prev.findIndex((m) => {
                if ((m.from || m.role) !== agentId) return false;
                return isPlanMessage(m);
              });
              if (existingIdx >= 0) {
                const next = [...prev];
                next[existingIdx] = {
                  ...next[existingIdx],
                  text: planText,
                  timestampMs: Date.now(),
                  timestamp: new Date().toLocaleTimeString(),
                };
                return next;
              }
              return [...prev, {
                role: 'agent' as const,
                from: agentId,
                to: 'user',
                text: planText,
                timestamp: new Date().toLocaleTimeString(),
                timestampMs: Date.now(),
              }];
            });
          } else if (item.phase === 'subagent_spawned' && item.data) {
            const parentId = String(item.agent_id || '').toLowerCase();
            const subagentId = String(item.data.subagent_id || '');
            const task = String(item.data.task || '');
            if (subagentId && parentId) {
              subagentParentMapRef.current[subagentId.toLowerCase()] = parentId;
              subagentStatsRef.current[subagentId.toLowerCase()] = { toolCount: 0, contextTokens: 0 };
              const newEntry: SubagentTreeEntry = {
                subagentId,
                task,
                status: 'running',
                toolCount: 0,
                contextTokens: 0,
                currentActivity: null,
              };
              setChatMessages((prev) => {
                // Find parent's generating message and add the entry
                let targetIdx = -1;
                for (let i = prev.length - 1; i >= 0; i--) {
                  const m = prev[i];
                  if ((m.from || '').toLowerCase() === parentId && m.isGenerating) {
                    targetIdx = i;
                    break;
                  }
                }
                if (targetIdx < 0) {
                  for (let i = prev.length - 1; i >= 0; i--) {
                    const m = prev[i];
                    if ((m.from || '').toLowerCase() === parentId && m.role === 'agent') {
                      targetIdx = i;
                      break;
                    }
                  }
                }
                if (targetIdx < 0) return prev;
                const next = [...prev];
                const msg = next[targetIdx];
                const tree = msg.subagentTree ? [...msg.subagentTree] : [];
                tree.push(newEntry);
                next[targetIdx] = { ...msg, subagentTree: tree };
                return next;
              });
            }
          } else if (item.phase === 'subagent_result' && item.data) {
            const parentId = String(item.agent_id || '').toLowerCase();
            const subagentId = String(item.data.subagent_id || '');
            if (subagentId && parentId) {
              setChatMessages((prev) =>
                updateParentSubagentTree(prev, parentId, subagentId, (entry) => ({
                  ...entry,
                  status: 'done',
                  currentActivity: null,
                }))
              );
              // Clean up refs
              delete subagentParentMapRef.current[subagentId.toLowerCase()];
              delete subagentStatsRef.current[subagentId.toLowerCase()];
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

        if (item.kind === 'ask_user') {
          const { question_id, questions } = item.data || {};
          if (question_id && questions) {
            setPendingAskUser({
              questionId: question_id,
              agentId: String(item.agent_id || ''),
              questions,
            });
          }
          return;
        }

        if (item.kind === 'text_segment') {
          const agentId = String(item.agent_id || '');
          if (!agentId) return;
          // Skip subagent text segments
          if (subagentParentMapRef.current[agentId.toLowerCase()]) return;
          const segText = String(item.text || '').trim();
          if (!segText) return;
          setChatMessages((prev) => {
            const idx = findLastGeneratingMessageIndex(prev, agentId);
            if (idx < 0) return upsertGeneratingAgentMessage(prev, agentId, '');
            const next = [...prev];
            const msg = { ...next[idx] };
            msg.segments = [...(msg.segments || []), { type: 'text' as const, text: segText }];
            msg.liveText = '';
            next[idx] = msg;
            return next;
          });
          return;
        }

        if (item.kind === 'activity') {
          const agentId = String(item.agent_id || '');
          if (!agentId) return;
          const statusRaw = String(item.data?.status || '').trim();
          const nextStatus = normalizeAgentStatus(statusRaw);
          const statusText = String(item.text || '').trim();

          // Check if this agent is a known subagent — route activity to parent tree
          const parentIdFromData = item.data?.parent_id ? String(item.data.parent_id) : null;
          const parentIdForSubagent =
            subagentParentMapRef.current[agentId.toLowerCase()] ||
            (parentIdFromData ? parentIdFromData.toLowerCase() : null);

          if (parentIdForSubagent && statusRaw !== 'idle_prompt_triggered') {
            // Update tool count and current activity on the parent's subagent tree
            if (nextStatus === 'calling_tool' && item.phase !== 'done') {
              const stats = subagentStatsRef.current[agentId.toLowerCase()];
              if (stats) stats.toolCount += 1;
              setChatMessages((prev) =>
                updateParentSubagentTree(prev, parentIdForSubagent, agentId, (entry) => ({
                  ...entry,
                  toolCount: (stats?.toolCount ?? entry.toolCount + 1),
                  currentActivity: statusText || entry.currentActivity,
                }))
              );
            } else if (nextStatus === 'thinking' || nextStatus === 'model_loading') {
              setChatMessages((prev) =>
                updateParentSubagentTree(prev, parentIdForSubagent, agentId, (entry) => ({
                  ...entry,
                  currentActivity: statusText || (nextStatus === 'thinking' ? 'Thinking...' : 'Model loading...'),
                }))
              );
            } else if (nextStatus === 'idle') {
              setChatMessages((prev) =>
                updateParentSubagentTree(prev, parentIdForSubagent, agentId, (entry) => ({
                  ...entry,
                  currentActivity: null,
                }))
              );
            }
            // Don't create separate messages for subagent activity
            return;
          }

          // Capture idle_prompt_triggered events for Mission activity tab
          if (statusRaw === 'idle_prompt_triggered') {
            setIdlePromptEvents((prev) => {
              const evt: IdlePromptEvent = {
                agent_id: agentId,
                project_root: String(item.project_root || ''),
                timestamp: Date.now(),
              };
              const next = [evt, ...prev];
              return next.length > 100 ? next.slice(0, 100) : next;
            });
          }

          if (statusRaw) {
            // Don't update agent status badge on "done" lifecycle events — those describe
            // the PREVIOUS status ending, not the current state. The next "doing" or "idle"
            // event will set the correct status. Only update on "doing" or explicit "idle".
            if (item.phase !== 'done' || nextStatus === 'idle') {
              setAgentStatus((prev) => ({
                ...prev,
                [agentId]: nextStatus,
              }));
              setAgentStatusText((prev) => ({
                ...prev,
                [agentId]:
                  nextStatus === 'idle'
                    ? 'Idle'
                    : statusText.length > 0
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
          }

          // Track run start time on first activity for an agent
          if (!runStartTsRef.current[agentId]) {
            runStartTsRef.current[agentId] = Date.now();
          }

          // Only add tool call status lines as activity entries (not "Thinking"/"Model loading").
          if (statusText.length > 0 && item.phase !== 'done') {
            setChatMessages((prev) => {
              const updated = appendGeneratingActivity(prev, agentId, statusText);
              // Also track in segments for interleaved rendering
              const idx = findLastGeneratingMessageIndex(updated, agentId);
              if (idx < 0 || !updated[idx].segments) return updated;
              const next = [...updated];
              const msg = { ...next[idx] };
              const segs = [...(msg.segments || [])];
              const last = segs[segs.length - 1];
              if (last?.type === 'tools') {
                segs[segs.length - 1] = { ...last, entries: [...(last.entries || []), statusText] };
              } else {
                segs.push({ type: 'tools', entries: [statusText] });
              }
              msg.segments = segs;
              next[idx] = msg;
              return next;
            });
          } else if ((nextStatus === 'model_loading' || nextStatus === 'thinking') && item.phase !== 'done') {
            const placeholder = nextStatus === 'model_loading' ? 'Model loading...' : 'Thinking...';
            setChatMessages((prev) => {
              // When segments exist, don't overwrite msg.text with placeholder
              const idx = findLastGeneratingMessageIndex(prev, agentId);
              if (idx >= 0 && prev[idx].segments) return prev;
              return upsertGeneratingAgentMessage(prev, agentId, placeholder);
            });
          }

          // Only finalize on true idle (run ended) or explicit failure — NOT on
          // calling_tool/done which just means one tool finished, not the whole run.
          if (nextStatus === 'idle' || item.phase === 'failed') {
            const startTs = runStartTsRef.current[agentId];
            const elapsed = startTs ? Date.now() - startTs : undefined;
            const ctxTokens = latestContextTokensRef.current[agentId] || undefined;
            delete runStartTsRef.current[agentId];

            setChatMessages((prev) => {
              const idx = findLastGeneratingMessageIndex(prev, agentId);
              if (idx < 0) return prev;
              // Don't finalize if the message has running subagents
              const tree = prev[idx].subagentTree;
              if (tree && tree.some((e) => e.status === 'running')) return prev;
              // Only finalize status-only messages here; real token content
              // gets finalized by the 'message' handler to avoid premature cutoff.
              const msgText = prev[idx].text || '';
              if (msgText && !isStatusLineText(msgText)) return prev;
              const next = [...prev];
              const entries = Array.isArray(next[idx].activityEntries) ? next[idx].activityEntries : [];
              const nonTransient = entries.filter((e: string) => !isTransientStatus(e));
              next[idx] = {
                ...next[idx],
                isGenerating: false,
                activitySummary: summarizeActivityEntries(entries, false) || next[idx].activitySummary,
                toolCount: nonTransient.length || next[idx].toolCount,
                durationMs: elapsed || next[idx].durationMs,
                contextTokens: ctxTokens || next[idx].contextTokens,
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

          // Skip token events from subagents — their output is summarized in the parent tree
          if (subagentParentMapRef.current[agentId.toLowerCase()]) return;

          if (item.phase === 'done') {
            if (isThinking) {
              // When segments exist, don't set isThinking — text_segment handles the text
              setChatMessages((prev) => {
                const idx = findLastGeneratingMessageIndex(prev, agentId);
                if (idx >= 0) {
                  const next = [...prev];
                  if (next[idx].segments) {
                    // Segments mode: discard thinking flag, just keep generating
                    return next;
                  }
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
              // When segments mode is active, discard thinking tokens entirely.
              // The text_segment event captures this text more reliably.
              // Only show a non-thinking final-answer token stream in liveText.
              if (next[idx].segments) {
                if (isThinking) {
                  // Discard thinking tokens — don't accumulate into liveText
                  return prev;
                }
                next[idx] = {
                  ...next[idx],
                  liveText: (next[idx].liveText || '') + tokenText,
                  isGenerating: true,
                  isThinking: false,
                  timestampMs: Date.now(),
                };
                return next;
              }
              const isPlaceholder = isStatusLineText(next[idx].text || '');
              next[idx] = {
                ...next[idx],
                text: isPlaceholder ? tokenText : (next[idx].text || '') + tokenText,
                isGenerating: true,
                isThinking,
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
          let content = String(item.text || '');
          if (!content) return;
          if (shouldHideInternalChatMessage(from, content)) return;

          // Skip message events from subagents — their output is summarized in the parent tree
          if (subagentParentMapRef.current[from.toLowerCase()]) return;

          // Skip pure plan JSON messages — handled by plan_update SSE event
          try {
            const parsed = JSON.parse(content);
            if (parsed?.type === 'plan' && parsed?.plan) return;
          } catch (_e) { /* not JSON, continue */ }

          // Strip embedded plan JSON from agent text to avoid duplicate plan blocks
          if (from !== 'user') {
            content = stripEmbeddedStructuredJson(content);
            if (!content) return;
          }

          if (from !== 'user' && isStatusLineText(content)) {
            setChatMessages((prev) => appendGeneratingActivity(prev, from, content));
            return;
          }

          const tsMs = Number(item.ts_ms || Date.now());
          const msgStartTs = runStartTsRef.current[from];
          const msgElapsed = msgStartTs ? Date.now() - msgStartTs : undefined;
          const msgCtxTokens = latestContextTokensRef.current[from] || undefined;
          delete runStartTsRef.current[from];

          setChatMessages((prev) => {
            const generatingIdx = findLastGeneratingMessageIndex(prev, from);
            if (generatingIdx >= 0) {
              const next = [...prev];
              const existingEntries = Array.isArray(next[generatingIdx].activityEntries)
                ? next[generatingIdx].activityEntries
                : [];
              const nonTransient = existingEntries.filter((e: string) => !isTransientStatus(e));
              // Add final text as a text segment (if segments mode is active)
              let finalSegments = next[generatingIdx].segments;
              if (finalSegments && content.trim()) {
                const lastTextSeg = [...finalSegments].reverse().find(s => s.type === 'text');
                if (!lastTextSeg || lastTextSeg.text !== content.trim()) {
                  finalSegments = [...finalSegments, { type: 'text' as const, text: content.trim() }];
                }
              }
              next[generatingIdx] = {
                ...next[generatingIdx],
                text: content,
                to: to || next[generatingIdx].to || 'user',
                isGenerating: false,
                isThinking: false,
                liveText: undefined,
                segments: finalSegments,
                timestamp: new Date(tsMs).toLocaleTimeString(),
                timestampMs: tsMs,
                activitySummary:
                  summarizeActivityEntries(existingEntries, false) || next[generatingIdx].activitySummary,
                toolCount: nonTransient.length || next[generatingIdx].toolCount,
                durationMs: msgElapsed || next[generatingIdx].durationMs,
                contextTokens: msgCtxTokens || next[generatingIdx].contextTokens,
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

        if (item.kind === 'model_fallback') {
          const agentId = String(item.agent_id || '');
          const text = String(item.text || 'Model switched');
          setChatMessages((prev) => [
            ...prev,
            {
              role: 'agent' as const,
              from: 'system',
              to: '',
              text: `\u26A0\uFE0F ${text}`,
              timestamp: new Date().toLocaleTimeString(),
              timestampMs: Date.now(),
              isGenerating: false,
            },
          ]);
          if (agentId) {
            setAgentStatusText((prev) => ({
              ...prev,
              [agentId]: `Fallback: ${item.data?.actual_model || 'alternate model'}`,
            }));
          }
        }
      } catch (err) {
        console.error("SSE parse error", err);
        fetchWorkspaceState();
        fetchAgentRuns();
      }
    };

    return () => events.close();
  }, [currentPath, selectedProjectRoot, activeSessionId, fetchWorkspaceState, fetchFiles, fetchAllAgentTrees, fetchAgentRuns, shouldHideInternalChatMessage]);

  const sendChatMessage = async (userMessage: string, targetAgent?: string, images?: string[]) => {
    if (!userMessage.trim() && !(images && images.length > 0)) return;
    if (!selectedProjectRoot) return;
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
        ...(images && images.length > 0 ? { images } : {}),
      },
    ]);

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
          session_id: activeSessionId,
          ...(images && images.length > 0 ? { images } : {}),
        }),
      });
      const data = await resp.json();
      // If the server auto-created a session, adopt it
      if (data?.session_id && !activeSessionId) {
        setActiveSessionId(data.session_id);
        fetchSessions();
      }
      if (data?.status === 'queued') {
        // Remove the optimistically-added user message — it will appear in the queue banner instead
        setChatMessages((prev) => {
          const idx = prev.findLastIndex(
            (m) => m.role === 'user' && m.text === userMessage && m.to === agentToUse
          );
          if (idx < 0) return prev;
          const next = [...prev];
          next.splice(idx, 1);
          return next;
        });
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
    // Clear local state immediately — don't wait for API
    setChatMessages([]);
    setQueuedMessages([]);
    setActivePlan(null);
    setPendingPlan(null);
    setPendingPlanAgentId(null);
    // Suppress fetchWorkspaceState repopulation during cooldown
    chatClearTsRef.current = Date.now();
    try {
      const resp = await fetch('/api/chat/clear', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          session_id: activeSessionId,
        }),
      });
      if (!resp.ok) {
        addLog(`Clear chat API error: ${resp.status}`);
      }
    } catch (e) {
      addLog(`Error clearing chat: ${e}`);
    }
  };

  const respondToAskUser = async (questionId: string, answers: import('./types').AskUserAnswer[]) => {
    try {
      await fetch('/api/ask-user-response', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question_id: questionId, answers }),
      });
      setPendingAskUser(null);
    } catch (e) {
      addLog(`Error responding to AskUser: ${e}`);
    }
  };

  const approvePlan = async () => {
    if (!pendingPlanAgentId || !selectedProjectRoot) return;
    try {
      await fetch('/api/plan/approve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: pendingPlanAgentId,
          session_id: activeSessionId,
        }),
      });
      setPendingPlan(null);
      setPendingPlanAgentId(null);
    } catch (e) {
      addLog(`Error approving plan: ${e}`);
    }
  };

  const rejectPlan = async () => {
    if (!pendingPlanAgentId || !selectedProjectRoot) return;
    try {
      await fetch('/api/plan/reject', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: pendingPlanAgentId,
          session_id: activeSessionId,
        }),
      });
      setPendingPlan(null);
      setPendingPlanAgentId(null);
    } catch (e) {
      addLog(`Error rejecting plan: ${e}`);
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

      const body = displayMessages
        .map((m) => {
          const from = m.from || m.role;
          const to = m.to ? ` → ${m.to}` : '';
          const lines: string[] = [`[${m.timestamp}] ${from}${to}`];
          // Include subagent tree entries
          if (m.subagentTree && m.subagentTree.length > 0) {
            for (const sa of m.subagentTree) {
              const stats = [];
              if (sa.toolCount > 0) stats.push(`${sa.toolCount} tool uses`);
              if (sa.contextTokens > 0) stats.push(`${(sa.contextTokens / 1000).toFixed(1)}k tokens`);
              lines.push(`  [subagent:${sa.subagentId}] ${sa.task}${stats.length ? ` (${stats.join(', ')})` : ''} — ${sa.status}`);
            }
          }
          // Include tool activity entries
          const entries = Array.isArray(m.activityEntries) ? m.activityEntries : [];
          if (entries.length > 0) {
            for (const entry of entries) {
              lines.push(`  > ${entry}`);
            }
          } else if (m.activitySummary) {
            lines.push(`  > ${m.activitySummary}`);
          }
          if (m.text) lines.push(m.text);
          return lines.join('\n') + '\n';
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

  return (
    <>
    {currentPage === 'settings' && (
      <SettingsPage
        onBack={() => {
          setCurrentPage('main');
          setInitialSettingsTab(undefined);
          fetchModels();
          fetchOllamaStatus();
        }}
        projectRoot={selectedProjectRoot}
        initialTab={initialSettingsTab}
      />
    )}
    {currentPage === 'memory' && (
      <MemoryPage
        onBack={() => setCurrentPage('main')}
      />
    )}
    {currentPage === 'mission' && (
      <MissionPage
        onBack={() => setCurrentPage('main')}
        projectRoot={selectedProjectRoot}
        agents={agents}
        agentStatus={agentStatus}
        agentRunSummary={agentRunSummary}
        mission={mission}
        missionDraft={missionDraft}
        onMissionDraftChange={setMissionDraft}
        onSaveMission={saveMission}
        onClearMission={clearMission}
        idlePromptEvents={idlePromptEvents}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
      {/* Header */}
      <HeaderBar
        copyChat={copyChat}
        copyChatStatus={copyChatStatus}
        clearChat={clearChat}
        isRunning={isRunning}
        verboseMode={verboseMode}
        onToggleVerbose={() => setVerboseMode((v) => !v)}
        onOpenMission={() => setCurrentPage('mission')}
        missionActive={!!mission?.active}
        onOpenMemory={() => setCurrentPage('memory')}
        onOpenSettings={() => setCurrentPage('settings')}
      />

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Left: Sessions + Agents */}
        <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
          <SessionNav
            projects={projects}
            selectedProjectRoot={selectedProjectRoot}
            setSelectedProjectRoot={setSelectedProjectRoot}
            sessions={sessions}
            activeSessionId={activeSessionId}
            setActiveSessionId={setActiveSessionId}
            createSession={createSession}
            removeSession={removeSession}
            renameSession={renameSession}
            sessionCountsByProject={sessionCountsByProject}
            treesByProject={agentTreesByProject}
            onSelectPath={selectAgentPathFromTree}
            showAddProject={showAddProject}
            setShowAddProject={setShowAddProject}
            newProjectPath={newProjectPath}
            setNewProjectPath={setNewProjectPath}
            addProject={addProject}
            pickFolder={pickFolder}
            removeProject={removeProject}
          />
          <div className="border-t border-slate-200 dark:border-white/5">
            <CollapsibleCard
              title="AGENTS"
              icon={<Bot size={12} />}
              iconColor="text-blue-500"
              badge={`${mainAgents.length}`}
              defaultOpen
              headerAction={
                <button
                  onClick={() => setShowAgentSpecEditor(true)}
                  disabled={!selectedProjectRoot}
                  title="Edit agent markdown specs"
                  className="p-1 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 disabled:opacity-30 transition-colors"
                >
                  <FilePenLine size={12} />
                </button>
              }
            >
              <AgentsCard
                agents={mainAgents}
                workspaceState={workspaceState}
                isRunning={isRunning}
                selectedAgent={selectedAgent}
                setSelectedAgent={setSelectedAgent}
                agentStatus={agentStatus}
                agentStatusText={agentStatusText}
                agentWork={agentWork}
                agentRunSummary={agentRunSummary}
                agentContext={agentContext}
                projectRoot={selectedProjectRoot}
              />
            </CollapsibleCard>
          </div>
        </div>

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 p-2 min-h-0">
            <ChatPanel
              chatMessages={displayMessages}
              queuedMessages={queuedMessages}
              chatEndRef={chatEndRef}
              projectRoot={selectedProjectRoot}
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
              pendingPlan={pendingPlan}
              onApprovePlan={approvePlan}
              onRejectPlan={rejectPlan}
              pendingAskUser={pendingAskUser}
              onRespondToAskUser={respondToAskUser}
              verboseMode={verboseMode}
            />
          </div>
        </main>

        {/* Right: Mission, Models, Skills */}
        <aside className="w-72 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
          <CollapsibleCard
            title="MISSION"
            icon={<Target size={12} />}
            iconColor={mission ? 'text-green-500' : 'text-slate-400'}
            badge={mission ? 'Active' : 'None'}
            defaultOpen
          >
            <MissionSidebarCard
              mission={mission}
              projectRoot={selectedProjectRoot}
              onOpenMission={() => setCurrentPage('mission')}
            />
          </CollapsibleCard>
          <TaskListCard plan={activePlan} />
          <CollapsibleCard
            title="MODELS"
            icon={<Sparkles size={12} />}
            iconColor="text-purple-500"
            badge={`${models.length}`}
            defaultOpen
          >
            <ModelsCard
              models={models}
              agents={mainAgents}
              ollamaStatus={ollamaStatus}
              chatMessages={chatMessages}
              tokensPerSec={tokensPerSec}
              agentContext={agentContext}
            />
          </CollapsibleCard>
          <CollapsibleCard
            title="SKILLS"
            icon={<Zap size={12} />}
            iconColor="text-amber-500"
            badge={`${skills.length} loaded`}
            defaultOpen
          >
            <SkillsCard
              skills={skills}
              onManageSkills={() => {
                setInitialSettingsTab('skills');
                setCurrentPage('settings');
              }}
            />
          </CollapsibleCard>
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
