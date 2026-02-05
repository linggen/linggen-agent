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
  SessionInfo,
  SkillInfo,
} from './types';

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
  const [selectedAgent, setSelectedAgent] = useState<'lead' | 'coder'>('lead');
  const [isRunning, setIsRunning] = useState(false);
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  
  const [leadState, setLeadState] = useState<LeadState | null>(null);
  
  const chatEndRef = useRef<HTMLDivElement>(null);

  const addLog = (msg: string) => {
    setLogs(prev => [...prev, `[${new Date().toLocaleTimeString()}] ${msg}`]);
  };

  const chatMessageKey = (msg: ChatMessage) => {
    const from = msg.from || msg.role;
    const to = msg.to || '';
    return `${from}|${to}|${msg.text}`;
  };

  const mergeChatMessages = (persisted: ChatMessage[], live: ChatMessage[]) => {
    if (live.length === 0) return persisted;
    if (persisted.length === 0) return live;

    const lastPersisted = persisted[persisted.length - 1];
    const lastKey = chatMessageKey(lastPersisted);
    let lastIdx = -1;
    for (let i = live.length - 1; i >= 0; i -= 1) {
      if (chatMessageKey(live[i]) === lastKey) {
        lastIdx = i;
        break;
      }
    }

    const extras = lastIdx >= 0 ? live.slice(lastIdx + 1) : live.filter(m => m.isGenerating);
    const persistedKeys = new Set(persisted.map(chatMessageKey));
    const uniqueExtras = extras.filter(m => !persistedKeys.has(chatMessageKey(m)));
    return [...persisted, ...uniqueExtras];
  };

  // Chat scroll is user-controlled; no auto-scroll.

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
        const msgs: ChatMessage[] = data.messages.map(([meta, body]: any) => ({
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
          text: body,
          timestamp: new Date(meta.ts * 1000).toLocaleTimeString()
        }));
        setChatMessages(prev => mergeChatMessages(msgs, prev));
      } else {
        setChatMessages([]);
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
    }
  }, [selectedProjectRoot]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchLeadState();
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
    const events = new EventSource('/api/events');
    events.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data);
        if (event.type === 'StateUpdated') {
          fetchLeadState();
          fetchFiles(currentPath);
          fetchAgentTree();
        } else if (event.type === 'Observation') {
          // Observations are persisted to DB; refresh to show tool actions.
          fetchLeadState();
        } else if (event.type === 'Token') {
          setChatMessages(prev => {
            const lastMsg = prev[prev.length - 1];
            if (lastMsg && lastMsg.from === event.agent_id && lastMsg.isGenerating) {
              return prev.map((msg, idx) => 
                idx === prev.length - 1 
                  ? { ...msg, text: msg.text + event.token }
                  : msg
              );
            } else {
              return [...prev, {
                role: event.agent_id === 'lead' ? 'lead' : 'coder',
                from: event.agent_id,
                to: 'user',
                text: event.token,
                timestamp: new Date().toLocaleTimeString(),
                isGenerating: true
              }];
            }
          });
        } else if (event.type === 'Message') {
          setChatMessages(prev => {
            const lastMsg = prev[prev.length - 1];
            if (lastMsg && lastMsg.from === event.from && lastMsg.isGenerating) {
              return prev.map((msg, idx) => 
                idx === prev.length - 1 
                  ? { ...msg, text: event.content, isGenerating: false }
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

            return [...prev, {
              role,
              from: event.from,
              to: event.to,
              text: event.content,
              timestamp: new Date().toLocaleTimeString(),
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
  }, [currentPath]);

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

  const sendChatMessage = async (userMessage: string) => {
    if (!userMessage.trim() || !selectedProjectRoot) return;
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
      await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          project_root: selectedProjectRoot, 
          agent_id: selectedAgent, 
          message: userMessage,
          session_id: activeSessionId
        }),
      });
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
  };

  return (
    <div className="flex flex-col h-screen bg-slate-50 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
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
      />

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">
        
        {/* Left: File Tree */}
        <aside className="w-64 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f]">
          <div className="p-4 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
              <Activity size={14} /> Workspace
            </h2>
          </div>
          <AgentTree agentTree={agentTree} onSelect={readFile} />
        </aside>

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-50 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 p-4 min-h-0">
            <ChatPanel
              chatMessages={chatMessages}
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
        <aside className="w-80 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-50 dark:bg-[#0a0a0a] p-4 gap-4 overflow-y-auto">
          <AgentsCard agents={agents} leadState={leadState} isRunning={isRunning} selectedAgent={selectedAgent} />
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
