import React, { useState, useEffect, useRef } from 'react';
import { Activity } from 'lucide-react';
import { AgentTree } from './components/AgentTree';
import { AgentsCard } from './components/AgentsCard';
import { ModelsCard } from './components/ModelsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/ChatPanel';
import { HeaderBar } from './components/HeaderBar';
import type {
  AgentInfo,
  AgentTreeItem,
  ChatMessage,
  FileEntry,
  LeadState,
  ModelInfo,
  OllamaPsResponse,
  ProjectInfo,
  QueuedChatItem,
  SessionInfo,
  SkillInfo,
} from './types';

const SELECTED_AGENT_STORAGE_KEY = 'linggen-agent:selected-agent';
const LIVE_MESSAGE_GRACE_MS = 10_000;

const parseToolNameFromParsedPayload = (parsed: any): string | null => {
  if (!parsed || typeof parsed !== 'object') return null;
  if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') return parsed.tool;
  if (
    typeof parsed?.type === 'string' &&
    parsed.type !== 'ask' &&
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
  const cleaned = text
    .split('\n')
    .map((line) => line.trimEnd())
    .filter((line) => !parseToolNameFromMessage(line.trim()))
    .join('\n')
    .trim();
  return cleaned;
};

const isToolResultMessage = (from?: string, text?: string) => {
  return from === 'system' && !!text && text.startsWith('Tool ');
};

const App: React.FC = () => {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [selectedProjectRoot, setSelectedProjectRoot] = useState<string>('');
  const [agentTree, setAgentTree] = useState<Record<string, AgentTreeItem>>({});
  const [newProjectPath, setNewProjectPath] = useState('');
  const [showAddProject, setShowAddProject] = useState(false);

  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);

  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  const [task, setTask] = useState('');
  const [logs, setLogs] = useState<string[]>([]);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<'lead' | 'coder'>(() => {
    if (typeof window === 'undefined') return 'lead';
    const stored = window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY);
    return stored === 'coder' ? 'coder' : 'lead';
  });
  const [currentMode, setCurrentMode] = useState<'chat' | 'auto'>('auto');
  const [isRunning, setIsRunning] = useState(false);
  const [agentStatus, setAgentStatus] = useState<Record<string, 'idle' | 'thinking' | 'calling_tool' | 'working'>>({});
  const [agentStatusText, setAgentStatusText] = useState<Record<string, string>>({});
  const [queuedMessages, setQueuedMessages] = useState<QueuedChatItem[]>([]);
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  
  const [leadState, setLeadState] = useState<LeadState | null>(null);
  
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastSseSeqRef = useRef(0);

  const addLog = (msg: string) => {
    setLogs(prev => [...prev, `[${new Date().toLocaleTimeString()}] ${msg}`]);
  };

  const shouldHideInternalChatMessage = (from?: string, text?: string) => {
    if (!text) return false;
    if (isToolResultMessage(from, text)) return true;
    const toolName = parseToolNameFromMessage(text);
    if (toolName) return true;
    if (from !== 'system') return false;
    return text.startsWith('Starting autonomous loop for task:');
  };

  const chatMessageKey = (msg: ChatMessage) => {
    const from = msg.from || msg.role;
    const to = msg.to || '';
    const ts = msg.timestampMs ?? 0;
    return `${from}|${to}|${ts}|${msg.text}`;
  };

  const sameMessageContent = (a: ChatMessage, b: ChatMessage) => {
    const fromA = a.from || a.role;
    const fromB = b.from || b.role;
    const toA = a.to || '';
    const toB = b.to || '';
    return fromA === fromB && toA === toB && a.text === b.text;
  };

  const isStructuredAgentMessage = (msg: ChatMessage) => {
    if ((msg.from || msg.role) === 'user') return false;
    try {
      const parsed = JSON.parse(msg.text);
      return typeof parsed?.type === 'string';
    } catch (_e) {
      return false;
    }
  };

  const likelySameMessage = (a: ChatMessage, b: ChatMessage) => {
    if (!sameMessageContent(a, b)) return false;
    if (isStructuredAgentMessage(a) || isStructuredAgentMessage(b)) return true;
    const ta = a.timestampMs ?? 0;
    const tb = b.timestampMs ?? 0;
    if (ta === 0 || tb === 0) return true;
    return Math.abs(ta - tb) <= 120_000;
  };

  const mergeChatMessages = (persisted: ChatMessage[], live: ChatMessage[]) => {
    if (persisted.length === 0) return live;
    if (live.length === 0) return persisted;

    const now = Date.now();
    const uniqueExtras = live.filter(
      (m) => {
        if (m.isGenerating) return true;
        if (persisted.some((p) => likelySameMessage(p, m))) return false;
        const ts = m.timestampMs ?? now;
        // Keep non-persisted live messages briefly to bridge DB/state lag.
        return now - ts <= LIVE_MESSAGE_GRACE_MS;
      }
    );
    const merged = [...persisted, ...uniqueExtras];
    const seen = new Set<string>();
    return merged.filter((m) => {
      const key = chatMessageKey(m);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  };

  useEffect(() => {
    if (chatMessages.length > lastChatCountRef.current) {
      chatEndRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
    }
    lastChatCountRef.current = chatMessages.length;
  }, [chatMessages.length]);

  const fetchProjects = async () => {
    try {
      const resp = await fetch('/api/projects');
      const data = await resp.json();
      setProjects(data);
      if (data.length > 0 && !selectedProjectRoot) {
        setSelectedProjectRoot(data[0].path);
      }
    } catch (e) {
      addLog(`Error fetching projects: ${e}`);
    }
  };

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

  const fetchAgentTree = async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/workspace/tree?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      const data = await resp.json();
      setAgentTree(data);
    } catch (e) {
      addLog(`Error fetching agent tree: ${e}`);
    }
  };

  const fetchFiles = async (path = '') => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/files?project_root=${encodeURIComponent(selectedProjectRoot)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      setFiles(data);
      setCurrentPath(path);
    } catch (e) {
      addLog(`Error fetching files: ${e}`);
    }
  };

  const readFile = async (path: string) => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(selectedProjectRoot)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      setSelectedFileContent(data.content);
      setSelectedFilePath(path);
    } catch (e) {
      addLog(`Error reading file: ${e}`);
    }
  };

  const closeFilePreview = () => {
    setSelectedFilePath(null);
    setSelectedFileContent(null);
  };

  const fetchLeadState = async () => {
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
  };

  const fetchSessions = async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      const data = await resp.json();
      setSessions(data);
    } catch (e) {
      console.error('Failed to fetch sessions:', e);
    }
  };

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

  const fetchSkills = async () => {
    try {
      const resp = await fetch('/api/skills');
      const data = await resp.json();
      setSkills(data);
    } catch (e) {
      console.error('Failed to fetch skills:', e);
    }
  };

  const fetchAgents = async () => {
    try {
      const resp = await fetch('/api/agents');
      const data = await resp.json();
      setAgents(data);
    } catch (e) {
      console.error('Failed to fetch agents:', e);
    }
  };

  const fetchModels = async () => {
    try {
      const resp = await fetch('/api/models');
      const data = await resp.json();
      setModels(data);
    } catch (e) {
      console.error('Failed to fetch models:', e);
    }
  };

  const fetchOllamaStatus = async () => {
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) {
        const data = await resp.json();
        setOllamaStatus(data);
      }
    } catch (e) {
      console.error('Failed to fetch Ollama status:', e);
    }
  };

  const fetchSettings = async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/settings?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      if (!resp.ok) return;
      const data = await resp.json();
      setCurrentMode(data.mode === 'chat' ? 'chat' : 'auto');
    } catch (e) {
      console.error('Failed to fetch settings:', e);
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
  }, []);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchFiles();
      fetchLeadState();
      fetchAgentTree();
      fetchSessions();
      fetchSettings();
      setAgentStatus({});
      setAgentStatusText({});
      setQueuedMessages([]);
    }
  }, [selectedProjectRoot]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchLeadState();
      setQueuedMessages([]);
    }
  }, [activeSessionId]);

  useEffect(() => {
    if (!selectedProjectRoot) return;
    const interval = window.setInterval(() => {
      fetchLeadState();
    }, 2000);
    return () => window.clearInterval(interval);
  }, [selectedProjectRoot, activeSessionId]);

  useEffect(() => {
    window.localStorage.setItem(SELECTED_AGENT_STORAGE_KEY, selectedAgent);
  }, [selectedAgent]);

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
          fetchAgentTree();
        } else if (event.type === 'AgentStatus') {
          const nextStatus: 'idle' | 'thinking' | 'calling_tool' | 'working' =
            event.status === 'calling_tool'
              ? 'calling_tool'
              : event.status === 'thinking'
                ? 'thinking'
                : event.status === 'working'
                  ? 'working'
                  : 'idle';
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
                  : nextStatus === 'thinking'
                    ? 'Thinking'
                    : nextStatus === 'working'
                      ? 'Working'
                      : 'Idle',
          }));
          if (nextStatus === 'thinking' || nextStatus === 'calling_tool') {
            const placeholder = nextStatus === 'calling_tool'
              ? `Calling tool: ${event.detail || '...'}` 
              : 'Thinking...';
            setChatMessages(prev => {
              const lastMsg = prev[prev.length - 1];
              if (lastMsg && lastMsg.from === event.agent_id && lastMsg.isGenerating) {
                return prev.map((msg, idx) =>
                  idx === prev.length - 1
                    ? { ...msg, text: placeholder }
                    : msg
                );
              }
              return prev;
            });
          }
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
            const lastMsg = prev[prev.length - 1];
            if (lastMsg && lastMsg.from === event.agent_id && lastMsg.isGenerating) {
              const nextText = lastMsg.text === 'Thinking...' ? event.token : lastMsg.text + event.token;
              const rawToolName = extractToolNameFromRawText(nextText);
              if (rawToolName) {
                const narrative = stripToolPayloadLines(nextText);
                const finalizedPrev = [...prev];
                if (finalizedPrev.length > 0) {
                  const lastIdx = finalizedPrev.length - 1;
                  if (narrative.length > 0 && finalizedPrev[lastIdx].text !== 'Thinking...') {
                    finalizedPrev[lastIdx] = {
                      ...finalizedPrev[lastIdx],
                      text: narrative,
                      isGenerating: false,
                    };
                  } else {
                    finalizedPrev.pop();
                  }
                }
                return [
                  ...finalizedPrev,
                  {
                    role: event.agent_id === 'lead' ? 'lead' : 'coder',
                    from: event.agent_id,
                    to: 'user',
                    text: `Calling tool: ${rawToolName}`,
                    timestamp: new Date().toLocaleTimeString(),
                    timestampMs: Date.now(),
                    isGenerating: false,
                  },
                ];
              }
              return prev.map((msg, idx) => 
                idx === prev.length - 1 
                  ? {
                      ...msg,
                      text: rawToolName ? `Calling tool: ${rawToolName}` : nextText,
                    }
                  : msg
              );
            } else {
              const rawToolName = extractToolNameFromRawText(event.token);
              return [...prev, {
                role: event.agent_id === 'lead' ? 'lead' : 'coder',
                from: event.agent_id,
                to: 'user',
                text: rawToolName ? `Calling tool: ${rawToolName}` : event.token,
                timestamp: new Date().toLocaleTimeString(),
                timestampMs: Date.now(),
                isGenerating: true
              }];
            }
          });
        } else if (event.type === 'Message') {
          const toolName = parseToolNameFromMessage(event.content || '');
          if (toolName) {
            setChatMessages(prev => {
              const lastMsg = prev[prev.length - 1];
              if (lastMsg && lastMsg.from === event.from && lastMsg.isGenerating) {
                const narrative = stripToolPayloadLines(lastMsg.text);
                const head = prev.slice(0, -1);
                const finalizedNarrative =
                  narrative.length > 0 && lastMsg.text !== 'Thinking...'
                    ? [{ ...lastMsg, text: narrative, isGenerating: false }]
                    : [];
                return [
                  ...head,
                  ...finalizedNarrative,
                  {
                    role: event.from === 'lead' ? 'lead' : 'coder',
                    from: event.from,
                    to: event.to || 'user',
                    text: `Calling tool: ${toolName}`,
                    timestamp: new Date().toLocaleTimeString(),
                    timestampMs: Date.now(),
                    isGenerating: false,
                  },
                ];
              }
              return [...prev, {
                role: event.from === 'lead' ? 'lead' : 'coder',
                from: event.from,
                to: event.to || 'user',
                text: `Calling tool: ${toolName}`,
                timestamp: new Date().toLocaleTimeString(),
                timestampMs: Date.now(),
                isGenerating: false,
              }];
            });
            return;
          }
          if (shouldHideInternalChatMessage(event.from, event.content)) {
            return;
          }
          const cleanedContent =
            event.from === 'user'
              ? (event.content || '')
              : stripToolPayloadLines(event.content || '');
          if (event.from !== 'user' && !cleanedContent) {
            return;
          }
          if (event.from && event.from !== 'user') {
          }
          setChatMessages(prev => {
            const lastMsg = prev[prev.length - 1];
            if (lastMsg && lastMsg.from === event.from && lastMsg.isGenerating) {
              return prev.map((msg, idx) => 
                idx === prev.length - 1 
                  ? { ...msg, text: cleanedContent, isGenerating: false }
                  : msg
              );
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
        }
      } catch (err) {
        // If we ever receive a malformed SSE payload (e.g. due to lag/drop),
        // fall back to a state refresh so tool actions still show up.
        console.error("SSE parse error", err);
        fetchLeadState();
      }
    };

    return () => events.close();
  }, [currentPath, selectedProjectRoot, activeSessionId]);

  const handleRun = async () => {
    if (!selectedProjectRoot) return;
    setIsRunning(true);
    addLog(`Running ${selectedAgent.toUpperCase()} agent...`);
    try {
      const resp = await fetch('/api/run', { 
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, agent_id: selectedAgent }),
      });
      const data = await resp.json();
      addLog(`Result: ${JSON.stringify(data).substring(0, 100)}...`);
    } catch (e) {
      addLog(`Error: ${e}`);
    }
    setIsRunning(false);
  };

  const handleSetTask = async () => {
    if (!task.trim() || !selectedProjectRoot) return;
    addLog(`Setting task for ${selectedAgent}: ${task}`);
    await fetch('/api/task', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: selectedProjectRoot, agent_id: selectedAgent, task }),
    });
    setTask('');
  };

  const sendChatMessage = async (userMessage: string, targetAgent?: 'lead' | 'coder') => {
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
      setAgentStatus((prev) => ({ ...prev, [agentToUse]: 'thinking' }));
      setAgentStatusText((prev) => ({ ...prev, [agentToUse]: 'Thinking' }));
      setChatMessages(prev => [
        ...prev,
        {
          role: agentToUse,
          from: agentToUse,
          to: 'user',
          text: 'Thinking...',
          timestamp: new Date().toLocaleTimeString(),
          timestampMs: Date.now(),
          isGenerating: true,
        },
      ]);
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
    fetchAgentTree();
    fetchSessions();
    fetchSettings();
  };

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
      {/* Header */}
      <HeaderBar
        projects={projects}
        selectedProjectRoot={selectedProjectRoot}
        setSelectedProjectRoot={setSelectedProjectRoot}
        showAddProject={showAddProject}
        setShowAddProject={setShowAddProject}
        newProjectPath={newProjectPath}
        setNewProjectPath={setNewProjectPath}
        addProject={addProject}
        removeProject={removeProject}
        pickFolder={pickFolder}
        refreshPageState={refreshPageState}
        isRunning={isRunning}
        currentMode={currentMode}
        onModeChange={updateMode}
      />

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">
        
        {/* Left: File Tree */}
        <aside className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f]">
          <div className="p-4 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
              <Activity size={14} /> Workspace
            </h2>
          </div>
          <AgentTree agentTree={agentTree} onSelect={readFile} />
        </aside>

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 p-4 min-h-0">
            <ChatPanel
              chatMessages={chatMessages}
              queuedMessages={queuedMessages}
              chatEndRef={chatEndRef}
              copyChat={copyChat}
              copyChatStatus={copyChatStatus}
              clearChat={clearChat}
              createSession={createSession}
              removeSession={removeSession}
              sessions={sessions}
              activeSessionId={activeSessionId}
              setActiveSessionId={setActiveSessionId}
              selectedAgent={selectedAgent}
              setSelectedAgent={setSelectedAgent}
              skills={skills}
              agents={agents}
              onSendMessage={sendChatMessage}
            />
          </div>
        </main>

        {/* Right: Status */}
        <aside className="w-80 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-4 gap-4 overflow-y-auto">
          <AgentsCard
            agents={agents}
            leadState={leadState}
            isRunning={isRunning}
            selectedAgent={selectedAgent}
            agentStatus={agentStatus}
            agentStatusText={agentStatusText}
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
