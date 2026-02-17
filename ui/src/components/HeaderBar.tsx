import React from 'react';
import { Copy, Eraser, Plus, Settings, Sparkles } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, SessionInfo } from '../types';

const contextPercent = (tokens: number, tokenLimit?: number) => {
  const limit = tokenLimit && tokenLimit > 0 ? tokenLimit : undefined;
  if (!limit || !tokens || tokens <= 0) return null;
  return Math.round((tokens / limit) * 100);
};

const formatCompactInt = (n: number) => {
  if (!Number.isFinite(n)) return '';
  if (n >= 1_000_000) return `${Math.round(n / 100_000) / 10}m`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${Math.round(n / 100) / 10}k`;
  return `${n}`;
};

export const HeaderBar: React.FC<{
  showAddProject: boolean;
  newProjectPath: string;
  setNewProjectPath: (value: string) => void;
  addProject: () => void;
  pickFolder: () => void;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  mainAgents: AgentInfo[];
  agentStatus?: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>;
  sessions: SessionInfo[];
  activeSessionId: string | null;
  setActiveSessionId: (value: string | null) => void;
  createSession: () => void;
  copyChat: () => void;
  copyChatStatus: 'idle' | 'copied' | 'error';
  clearChat: () => void;
  removeSession: (id: string) => void;
  isRunning: boolean;
  currentMode: 'chat' | 'auto';
  onModeChange: (mode: 'chat' | 'auto') => void;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onOpenSettings?: () => void;
}> = ({
  showAddProject,
  newProjectPath,
  setNewProjectPath,
  addProject,
  pickFolder,
  selectedAgent,
  setSelectedAgent,
  mainAgents,
  agentStatus,
  sessions,
  activeSessionId,
  setActiveSessionId,
  createSession,
  copyChat,
  copyChatStatus,
  clearChat,
  removeSession,
  isRunning,
  currentMode,
  onModeChange,
  agentContext,
  onOpenSettings,
}) => {
  return (
    <header className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.6fr)_minmax(0,1fr)] items-center gap-3 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
      <div className="flex items-center gap-6 min-w-0">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 bg-blue-600 rounded-lg flex items-center justify-center shadow-lg shadow-blue-600/20">
            <Sparkles size={18} className="text-white" />
          </div>
          <h1 className="text-lg font-bold tracking-tight text-slate-900 dark:text-white truncate">Linggen Agent</h1>
        </div>
      </div>

      <div className="flex items-center gap-1.5 justify-self-center min-w-0 overflow-x-auto">
        {mainAgents.map((agent) => {
          const id = agent.name.toLowerCase();
          const isSelected = id === selectedAgent.toLowerCase();
          const status = agentStatus?.[id] || 'idle';
          const context = agentContext?.[id];
          const pct = context ? contextPercent(context.tokens, context.tokenLimit) : null;
          return (
            <button
              key={agent.name}
              onClick={() => setSelectedAgent(id)}
              className={cn(
                'px-2.5 py-1 rounded-full text-[10px] font-bold uppercase tracking-wide border transition-colors whitespace-nowrap group relative',
                isSelected
                  ? 'bg-blue-600 text-white border-blue-600'
                  : 'bg-white dark:bg-black/20 text-slate-600 dark:text-slate-300 border-slate-200 dark:border-white/10 hover:bg-slate-100 dark:hover:bg-white/5'
              )}
            >
              <div className="flex items-center gap-1.5">
                {agent.name}
                <span
                  className={cn(
                    'px-1.5 py-0.5 rounded-full text-[9px]',
                    status === 'working'
                      ? 'bg-green-500/15 text-green-600 dark:text-green-300'
                      : status === 'thinking'
                        ? 'bg-blue-500/15 text-blue-600 dark:text-blue-300'
                        : status === 'calling_tool'
                          ? 'bg-amber-500/15 text-amber-700 dark:text-amber-300'
                          : status === 'model_loading'
                            ? 'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300'
                            : 'bg-slate-500/15 text-slate-600 dark:text-slate-300'
                  )}
                >
                  {status}
                </span>
                {context && (
                <span
                  className="text-[9px] opacity-60 font-medium"
                  title={
                    pct === null
                      ? `${context.tokens.toLocaleString()} tokens • ${context.messages} msgs`
                      : `${context.tokens.toLocaleString()} tokens • ${context.messages} msgs (model max ctx ~${Number(context.tokenLimit).toLocaleString()} tokens)`
                  }
                >
                  {pct === null
                    ? ''
                    : `${pct}% (${formatCompactInt(context.tokens)} / ${formatCompactInt(Number(context.tokenLimit))})`}
                </span>
                )}
              </div>
            </button>
          );
        })}

        <select
          value={activeSessionId || ''}
          onChange={(e) => setActiveSessionId(e.target.value || null)}
          className="text-[10px] bg-white dark:bg-black/20 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none w-40 shrink-0"
        >
          <option value="">Default Session</option>
          {sessions.map((s) => (
            <option key={s.id} value={s.id}>
              {s.title}
            </option>
          ))}
        </select>

        <button
          onClick={createSession}
          className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-md text-blue-500 transition-colors shrink-0"
          title="New Session"
        >
          <Plus size={14} />
        </button>
        <button
          onClick={copyChat}
          className={cn(
            'p-1.5 rounded-md transition-colors text-slate-500 shrink-0',
            copyChatStatus === 'copied'
              ? 'bg-green-500/10 text-green-600'
              : copyChatStatus === 'error'
                ? 'bg-red-500/10 text-red-500'
                : 'hover:bg-slate-100 dark:hover:bg-white/5'
          )}
          title={
            copyChatStatus === 'copied'
              ? 'Copied'
              : copyChatStatus === 'error'
                ? 'Copy failed'
                : 'Copy Chat'
          }
        >
          <Copy size={14} />
        </button>
        <button
          onClick={clearChat}
          className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-md text-slate-500 transition-colors shrink-0"
          title="Clear Chat"
        >
          <Eraser size={14} />
        </button>
        {activeSessionId && (
          <button
            onClick={() => removeSession(activeSessionId)}
            className="text-[10px] text-red-500 hover:underline shrink-0"
          >
            Delete
          </button>
        )}
      </div>

      {showAddProject && (
        <div className="absolute top-14 left-1/2 -translate-x-1/2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-xl p-4 shadow-2xl z-[60] flex flex-col gap-3 w-[min(42rem,90vw)]">
          <div className="flex gap-2">
            <input
              value={newProjectPath}
              onChange={(e) => setNewProjectPath(e.target.value)}
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
          <button
            onClick={addProject}
            className="w-full py-2 bg-blue-600 text-white rounded-lg text-[10px] font-bold shadow-lg shadow-blue-600/20"
          >
            Add Project
          </button>
        </div>
      )}

      <div className="flex items-center gap-4 bg-slate-100 dark:bg-white/5 px-3 py-1.5 rounded-full border border-slate-200 dark:border-white/10 shadow-sm justify-self-end">
        <div className="flex items-center gap-2">
          <div className={cn('w-2 h-2 rounded-full', isRunning ? 'bg-green-500 animate-pulse' : 'bg-slate-400')} />
          <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">{isRunning ? 'Active' : 'Standby'}</span>
        </div>
        <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
        <select
          value={currentMode}
          onChange={(e) => onModeChange((e.target.value === 'chat' ? 'chat' : 'auto'))}
          className="text-[10px] font-bold text-blue-600 dark:text-blue-400 uppercase tracking-widest bg-transparent outline-none"
          title="Prompt mode"
        >
          <option value="auto">Mode: Auto</option>
          <option value="chat">Mode: Chat</option>
        </select>
        {onOpenSettings && (
          <>
            <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
            <button
              onClick={onOpenSettings}
              className="p-1 hover:text-blue-500 text-slate-500 transition-colors"
              title="Settings"
            >
              <Settings size={14} />
            </button>
          </>
        )}
      </div>
    </header>
  );
};
