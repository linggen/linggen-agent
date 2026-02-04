import React, { useState, useEffect, useRef } from 'react';
import { Terminal, Play, Eye, Activity, Send, MessageSquare, ListTodo, FileText, Folder, ChevronRight, ChevronDown, User, Bot, Sparkles, Trash2, Plus, RefreshCw, Eraser, Copy } from 'lucide-react';
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

interface ChatMessage {
  role: 'user' | 'agent' | 'lead' | 'coder';
  from?: string;
  to?: string;
  text: string;
  timestamp: string;
  isGenerating?: boolean;
}

interface FileEntry {
  name: string;
  isDir: boolean;
  path: string;
}

interface LeadState {
  active_lead_task: [any, string] | null;
  user_stories: [any, string] | null;
  tasks: [any, string][];
  messages: [any, string][];
}

interface ProjectInfo {
  path: string;
  name: string;
  added_at: number;
}

interface AgentTreeItem {
  type: 'file' | 'dir';
  agent?: string;
  status?: string;
  path?: string;
  children?: Record<string, AgentTreeItem>;
}

interface SkillInfo {
  name: string;
  description: string;
  source: { type: string };
}

interface AgentInfo {
  name: string;
  description: string;
}

interface ModelInfo {
  id: string;
  provider: string;
  model: string;
  url: string;
}

interface OllamaPsModel {
  name: string;
  model: string;
  size: number;
  size_vram: number;
  details: {
    parameter_size: string;
    quantization_level: string;
  };
}

interface OllamaPsResponse {
  models: OllamaPsModel[];
}

interface SessionInfo {
  id: string;
  repo_path: string;
  title: string;
  created_at: number;
}

const TreeNode: React.FC<{ name: string, item: AgentTreeItem, depth?: number, onSelect: (path: string) => void }> = ({ name, item, depth = 0, onSelect }) => {
  const [isOpen, setIsOpen] = useState(true);
  
  if (item.type === 'file') {
    return (
      <button 
        onClick={() => onSelect(item.path || name)}
        className="w-full flex items-center justify-between px-2 py-1 rounded text-xs hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        <div className="flex items-center gap-2 truncate">
          <FileText size={12} className="text-slate-400 shrink-0" />
          <span className="truncate">{name}</span>
        </div>
        {item.agent && (
          <span className={cn(
            "text-[8px] font-bold px-1.5 py-0.5 rounded-full uppercase tracking-tighter shrink-0 ml-2",
            item.status === 'working' ? "bg-blue-500/20 text-blue-500" : "bg-slate-500/20 text-slate-500"
          )}>
            {item.agent} {item.status === 'working' ? '...' : ''}
          </span>
        )}
      </button>
    );
  }

  return (
    <div>
      <button 
        onClick={() => setIsOpen(!isOpen)}
        className="w-full flex items-center gap-2 px-2 py-1 rounded text-xs hover:bg-slate-100 dark:hover:bg-white/5 transition-colors text-slate-500"
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <Folder size={12} className="text-blue-400 shrink-0" />
        <span className="truncate font-bold">{name}</span>
      </button>
      {isOpen && item.children && (
        <div className="flex flex-col">
          {Object.entries(item.children).map(([childName, childItem]) => (
            <TreeNode 
              key={childName} 
              name={childName} 
              item={childItem} 
              depth={depth + 1} 
              onSelect={onSelect} 
            />
          ))}
        </div>
      )}
    </div>
  );
};

const App: React.FC = () => {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [selectedProjectRoot, setSelectedProjectRoot] = useState<string>('');
  const [agentTree, setAgentTree] = useState<Record<string, AgentTreeItem>>({});
  const [newProjectPath, setNewProjectPath] = useState('');
  const [showAddProject, setShowAddProject] = useState(false);

  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [showSkillDropdown, setShowSkillDropdown] = useState(false);
  const [skillFilter, setSkillFilter] = useState('');

  const [showAgentDropdown, setShowAgentDropdown] = useState(false);
  const [agentFilter, setAgentFilter] = useState('');
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);

  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  const [task, setTask] = useState('');
  const [logs, setLogs] = useState<string[]>([]);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [chatInput, setChatInput] = useState('');
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

  const addChatMessage = (role: ChatMessage['role'], text: string, from?: string, to?: string) => {
    setChatMessages(prev => [...prev, {
      role,
      from,
      to,
      text,
      timestamp: new Date().toLocaleTimeString()
    }]);
  };

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [chatMessages]);

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
        setChatMessages(msgs);
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

  const sendChat = async () => {
    if (!chatInput.trim() || !selectedProjectRoot) return;
    const userMessage = chatInput;
    setChatInput('');
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);

    // Immediately show user message in UI
    setChatMessages(prev => [...prev, {
      role: 'user',
      from: 'user',
      to: selectedAgent,
      text: userMessage,
      timestamp: new Date().toLocaleTimeString()
    }]);
    
    // Handle slash commands
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
    if (!confirm('Clear chat history for this session?')) return;
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
      <header className="flex items-center justify-between px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/80 dark:bg-[#0f0f0f]/80 backdrop-blur-md z-50">
        <div className="flex items-center gap-6">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 bg-blue-600 rounded-lg flex items-center justify-center shadow-lg shadow-blue-600/20">
              <Sparkles size={18} className="text-white" />
            </div>
            <h1 className="text-lg font-bold tracking-tight text-slate-900 dark:text-white">Linggen Agent</h1>
          </div>

          <div className="flex items-center gap-2">
            <button 
              onClick={refreshPageState}
              disabled={!selectedProjectRoot || isRunning}
              className="p-1.5 hover:bg-blue-500/10 hover:text-blue-500 rounded-lg text-slate-500 transition-colors disabled:opacity-50"
              title="Refresh page state"
            >
              <RefreshCw size={16} className={cn(isRunning && "animate-spin")} />
            </button>
            <select 
              value={selectedProjectRoot}
              onChange={(e) => setSelectedProjectRoot(e.target.value)}
              className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-1.5 outline-none font-mono max-w-[300px]"
            >
              {projects.map(p => (
                <option key={p.path} value={p.path}>{p.name} ({p.path})</option>
              ))}
            </select>
            <button 
              onClick={() => setShowAddProject(!showAddProject)}
              className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-slate-500"
              title="Manage Projects"
            >
              <Folder size={16} />
            </button>
            {selectedProjectRoot && (
              <button 
                onClick={() => removeProject(selectedProjectRoot)}
                className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-lg text-slate-500 transition-colors"
                title="Remove Current Project"
              >
                <Trash2 size={16} />
              </button>
            )}
          </div>
        </div>

        {showAddProject && (
          <div className="absolute top-14 left-64 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-xl p-4 shadow-2xl z-[60] flex flex-col gap-3 w-96">
            <div className="flex gap-2">
              <input 
                value={newProjectPath}
                onChange={e => setNewProjectPath(e.target.value)}
                placeholder="Full path to repository..."
                className="flex-1 bg-slate-100 dark:bg-white/5 border-none rounded-lg px-3 py-2 text-xs outline-none"
              />
              <button 
                onClick={pickFolder}
                className="px-3 py-2 bg-slate-200 dark:bg-white/10 rounded-lg text-[10px] font-bold hover:bg-slate-300 dark:hover:bg-white/20 transition-colors"
                title="Browse..."
              >
                Browse
              </button>
            </div>
            <button onClick={addProject} className="w-full py-2 bg-blue-600 text-white rounded-lg text-[10px] font-bold shadow-lg shadow-blue-600/20">Add Project</button>
          </div>
        )}
        
        <div className="flex items-center gap-4 bg-slate-100 dark:bg-white/5 px-3 py-1.5 rounded-full border border-slate-200 dark:border-white/10">
          <div className="flex items-center gap-2">
            <div className={cn(
              "w-2 h-2 rounded-full",
              isRunning ? "bg-green-500 animate-pulse" : "bg-slate-400"
            )} />
            <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
              {isRunning ? 'Active' : 'Standby'}
            </span>
          </div>
          <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
          <span className="text-[10px] font-bold text-blue-600 dark:text-blue-400 uppercase tracking-widest">L2 Autonomy</span>
        </div>
      </header>

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">
        
        {/* Left: File Tree */}
        <aside className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f]">
          <div className="p-4 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
              <Activity size={14} /> Workspace
            </h2>
          </div>
          <div className="flex-1 overflow-y-auto p-2">
            {Object.entries(agentTree).length === 0 && (
              <div className="p-4 text-xs text-slate-500 italic text-center">
                No files tracked yet. Agents will appear here as they work.
              </div>
            )}
            {Object.entries(agentTree).map(([name, item]) => (
              <TreeNode key={name} name={name} item={item} onSelect={readFile} />
            ))}
          </div>
        </aside>

        {/* Center: Cards & Editor */}
        <main className="flex-1 flex flex-col overflow-hidden p-4 gap-4 bg-slate-50 dark:bg-[#0a0a0a]">
          
          <div className="grid grid-cols-2 gap-4 h-1/3">
            {/* Agents Card */}
            <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
              <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-blue-500 flex items-center gap-2">
                  <User size={12} /> Agents Status
                </h3>
                <span className="text-[10px] text-slate-400">Swarm</span>
              </div>
              <div className="flex-1 p-4 overflow-y-auto text-xs space-y-4">
                {agents.map(agent => (
                  <div key={agent.name} className="flex items-center justify-between bg-slate-50 dark:bg-black/20 px-3 py-2 rounded-lg border border-slate-200 dark:border-white/5">
                    <div className="flex items-center gap-2">
                      {agent.name === 'lead' ? <User size={14} className="text-blue-500" /> : <Bot size={14} className="text-purple-500" />}
                      <span className="font-bold uppercase tracking-tight">{agent.name}</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className={cn(
                        "text-[8px] font-bold px-1.5 py-0.5 rounded-full uppercase",
                        (isRunning && selectedAgent === agent.name) ? "bg-green-500/20 text-green-500 animate-pulse" : "bg-slate-500/20 text-slate-500"
                      )}>
                        {(isRunning && selectedAgent === agent.name) ? 'Working' : 'Idle'}
                      </span>
                    </div>
                  </div>
                ))}
                
                <div className="pt-2 border-t border-slate-200 dark:border-white/5">
                  <div className="text-[10px] text-slate-500 mb-1 font-bold uppercase tracking-widest">Active Lead Task</div>
                  <div className="bg-slate-50 dark:bg-black/20 p-2 rounded border border-slate-200 dark:border-white/5 italic text-[10px] text-slate-400 truncate">
                    {leadState?.active_lead_task ? leadState.active_lead_task[1].substring(0, 100) + '...' : 'No active task'}
                  </div>
                </div>
              </div>
            </section>

            {/* Models Card */}
            <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
              <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-purple-500 flex items-center gap-2">
                  <Sparkles size={12} /> Models Status
                </h3>
                <span className="text-[10px] text-slate-400">Inference</span>
              </div>
              <div className="flex-1 p-4 overflow-y-auto text-xs space-y-3">
                {models.map(m => {
                  const isActive = ollamaStatus?.models.some(om => om.name.includes(m.model) || m.model.includes(om.name));
                  const activeInfo = ollamaStatus?.models.find(om => om.name.includes(m.model) || m.model.includes(om.name));
                  
                  return (
                    <div key={m.id} className="bg-slate-50 dark:bg-black/20 p-3 rounded-lg border border-slate-200 dark:border-white/5 space-y-2">
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2">
                          <div className={cn(
                            "w-1.5 h-1.5 rounded-full",
                            isActive ? "bg-green-500 animate-pulse" : "bg-slate-500"
                          )} />
                          <span className="font-mono font-bold">{m.model}</span>
                        </div>
                        <span className="text-[8px] text-slate-500 uppercase">{m.provider}</span>
                      </div>
                      
                      {activeInfo && (
                        <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[9px] text-slate-400 font-mono">
                          <div className="flex justify-between border-b border-white/5 pb-0.5">
                            <span>PARAMS:</span>
                            <span className="text-slate-200">{activeInfo.details.parameter_size}</span>
                          </div>
                          <div className="flex justify-between border-b border-white/5 pb-0.5">
                            <span>VRAM:</span>
                            <span className="text-slate-200">{(activeInfo.size_vram / 1024 / 1024 / 1024).toFixed(1)}GB</span>
                          </div>
                          <div className="flex justify-between border-b border-white/5 pb-0.5">
                            <span>QUANT:</span>
                            <span className="text-slate-200">{activeInfo.details.quantization_level}</span>
                          </div>
                          <div className="flex justify-between border-b border-white/5 pb-0.5">
                            <span>CONTEXT:</span>
                            <span className="text-slate-200">{chatMessages.reduce((acc, msg) => acc + msg.text.length, 0).toLocaleString()}</span>
                          </div>
                        </div>
                      )}
                      
                      {!activeInfo && (
                        <div className="text-[9px] text-slate-600 italic">Model is currently idle</div>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          </div>

          {/* File Preview / Editor Area */}
          <section className="flex-1 bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
            <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
              <div className="flex items-center gap-2">
                <FileText size={14} className="text-slate-400" />
                <span className="text-xs font-mono text-slate-600">{selectedFilePath || 'Select a file to preview'}</span>
              </div>
            </div>
            <div className="flex-1 p-4 font-mono text-xs overflow-auto bg-slate-900 text-slate-300 whitespace-pre">
              {selectedFileContent || '// No file selected'}
            </div>
          </section>
        </main>

        {/* Right: Unified Chat */}
        <section className="w-96 border-l border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f]">
          <div className="p-4 border-b border-slate-200 dark:border-white/5 flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
                <MessageSquare size={14} /> Unified Chat
              </h2>
              <div className="flex items-center gap-2">
                <button
                  onClick={copyChat}
                  className={cn(
                    "p-1.5 rounded-lg transition-colors text-slate-500",
                    copyChatStatus === 'copied'
                      ? "bg-green-500/10 text-green-600"
                      : copyChatStatus === 'error'
                        ? "bg-red-500/10 text-red-500"
                        : "hover:bg-slate-100 dark:hover:bg-white/5"
                  )}
                  title={
                    copyChatStatus === 'copied'
                      ? "Copied"
                      : copyChatStatus === 'error'
                        ? "Copy failed"
                        : "Copy Chat"
                  }
                >
                  <Copy size={16} />
                </button>
                <button
                  onClick={clearChat}
                  className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-lg text-slate-500 transition-colors"
                  title="Clear Chat"
                >
                  <Eraser size={16} />
                </button>
                <button 
                  onClick={createSession}
                  className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-blue-500 transition-colors"
                  title="New Session"
                >
                  <Plus size={16} />
                </button>
                <select 
                  value={selectedAgent} 
                  onChange={(e: any) => setSelectedAgent(e.target.value)}
                  className="text-[10px] bg-slate-100 dark:bg-white/5 border-none rounded px-2 py-1 outline-none"
                >
                  <option value="lead">Lead Agent</option>
                  <option value="coder">Coder Agent</option>
                </select>
              </div>
            </div>

            <div className="flex flex-col gap-1">
              <select
                value={activeSessionId || ''}
                onChange={(e) => setActiveSessionId(e.target.value || null)}
                className="text-[10px] bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none w-full"
              >
                <option value="">Default Session</option>
                {sessions.map(s => (
                  <option key={s.id} value={s.id}>{s.title}</option>
                ))}
              </select>
              {activeSessionId && (
                <button 
                  onClick={() => removeSession(activeSessionId)}
                  className="text-[8px] text-red-500 hover:underline text-right"
                >
                  Delete Session
                </button>
              )}
            </div>
          </div>
          
          <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-4 custom-scrollbar">
            {chatMessages.map((msg, i) => (
              <div key={i} className={cn(
                "flex flex-col gap-1 max-w-[95%]",
                msg.role === 'user' ? "self-end items-end" : "self-start items-start"
              )}>
                <div className="flex items-center gap-1 px-1">
                  <span className="text-[9px] font-bold uppercase tracking-tighter text-slate-500">
                    {msg.from || msg.role} {msg.to ? `→ ${msg.to}` : ''}
                  </span>
                </div>
                <div className={cn(
                  "px-3 py-2 rounded-xl text-xs leading-relaxed shadow-sm",
                  msg.role === 'user' 
                    ? "bg-blue-600 text-white rounded-tr-none" 
                    : (msg.from === 'lead' && msg.to === 'coder' 
                        ? "bg-amber-500/10 border border-amber-500/20 text-amber-200 italic" 
                        : "bg-slate-100 dark:bg-white/5 text-slate-800 dark:text-slate-200 border border-slate-200 dark:border-white/10 rounded-tl-none")
                )}>
                  {(() => {
                    if (msg.role === 'user') return msg.text;
                    try {
                      const parsed = JSON.parse(msg.text);
                      if (parsed.type === 'ask' && parsed.question) {
                        return parsed.question;
                      }
                      if (parsed.type === 'tool' && parsed.tool) {
                        return (
                          <div className="flex items-center gap-2 text-blue-500 italic">
                            <Activity size={12} className="animate-pulse" />
                            <span>Using tool: {parsed.tool}...</span>
                          </div>
                        );
                      }
                      if (parsed.type === 'finalize_task' && parsed.packet) {
                        return (
                          <div className="space-y-2">
                            <div className="font-bold text-blue-500">Task Finalized: {parsed.packet.title}</div>
                            <div className="text-[10px] opacity-80">{parsed.packet.user_stories.join(', ')}</div>
                          </div>
                        );
                      }
                      return msg.text;
                    } catch (e) {
                      return msg.text;
                    }
                  })()}
                  {msg.isGenerating && <span className="inline-block w-1.5 h-3.5 bg-blue-500 ml-1 animate-pulse align-middle" />}
                </div>
                <span className="text-[8px] text-slate-500 px-1">{msg.timestamp}</span>
              </div>
            ))}
            <div ref={chatEndRef} />
          </div>

          <div className="p-4 border-t border-slate-200 dark:border-white/5 space-y-3">
            <div className="flex gap-2 bg-slate-100 dark:bg-white/5 p-1 rounded-xl border border-slate-200 dark:border-white/10 relative">
              {showSkillDropdown && (
                <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
                  {skills
                    .filter(skill => 
                      skill.name.toLowerCase().includes(skillFilter) ||
                      skill.description.toLowerCase().includes(skillFilter)
                    )
                    .map(skill => (
                      <button
                        key={skill.name}
                        onClick={() => {
                          const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));
                          setChatInput(`${beforeSlash}/${skill.name} `);
                          setShowSkillDropdown(false);
                        }}
                        className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                      >
                        <div className="font-bold text-blue-500">/{skill.name}</div>
                        <div className="text-slate-500 text-[10px]">{skill.description}</div>
                      </button>
                    ))}
                  {skills.filter(s => s.name.toLowerCase().includes(skillFilter)).length === 0 && (
                    <div className="p-3 text-[10px] text-slate-500 italic">No matching skills found</div>
                  )}
                </div>
              )}
              {showAgentDropdown && (
                <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
                  {agents
                    .filter(agent => 
                      agent.name.toLowerCase().includes(agentFilter)
                    )
                    .map(agent => (
                      <button
                        key={agent.name}
                        onClick={() => {
                          const beforeAt = chatInput.substring(0, chatInput.lastIndexOf('@'));
                          const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                          setChatInput(`${beforeAt}@${label} `);
                          setShowAgentDropdown(false);
                          setSelectedAgent(agent.name.toLowerCase() as 'lead' | 'coder');
                        }}
                        className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                      >
                        <div className="font-bold text-purple-500">@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                        <div className="text-slate-500 text-[10px]">{agent.description}</div>
                      </button>
                    ))}
                </div>
              )}
              <input 
                value={chatInput}
                onChange={e => {
                  const val = e.target.value;
                  setChatInput(val);
                  if (val.includes('/') && !val.includes(' ', val.lastIndexOf('/'))) {
                    setSkillFilter(val.substring(val.lastIndexOf('/') + 1).toLowerCase());
                    setShowSkillDropdown(true);
                    setShowAgentDropdown(false);
                  } else if (val.includes('@') && !val.includes(' ', val.lastIndexOf('@'))) {
                    setAgentFilter(val.substring(val.lastIndexOf('@') + 1).toLowerCase());
                    setShowAgentDropdown(true);
                    setShowSkillDropdown(false);
                  } else {
                    setShowSkillDropdown(false);
                    setShowAgentDropdown(false);
                  }
                }}
                onKeyDown={e => {
                  if (e.key === 'Enter' && !showSkillDropdown && !showAgentDropdown) sendChat();
                  if (e.key === 'Escape') {
                    setShowSkillDropdown(false);
                    setShowAgentDropdown(false);
                  }
                }}
                placeholder="Message... (use / for skills, @ for agents)"
                className="flex-1 bg-transparent border-none px-3 py-2 text-xs outline-none"
              />
              <button onClick={sendChat} className="w-8 h-8 rounded-lg bg-blue-600 text-white flex items-center justify-center shadow-lg shadow-blue-600/20">
                <Send size={14} />
              </button>
            </div>
          </div>
        </section>
      </div>

      <style>{`
        .custom-scrollbar::-webkit-scrollbar { width: 4px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(255, 255, 255, 0.05); border-radius: 10px; }
      `}</style>
    </div>
  );
};

export default App;
