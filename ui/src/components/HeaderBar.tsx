import React from 'react';
import { Copy, Database, Eraser, Settings } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo } from '../types';

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
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  mainAgents: AgentInfo[];
  agentStatus?: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>;
  copyChat: () => void;
  copyChatStatus: 'idle' | 'copied' | 'error';
  clearChat: () => void;
  isRunning: boolean;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onOpenSettings?: () => void;
  onOpenMemory?: () => void;
}> = ({
  selectedAgent,
  setSelectedAgent,
  mainAgents,
  agentStatus,
  copyChat,
  copyChatStatus,
  clearChat,
  isRunning,
  agentContext,
  onOpenSettings,
  onOpenMemory,
}) => {
  return (
    <header className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.6fr)_minmax(0,1fr)] items-center gap-3 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
      <div className="flex items-center gap-6 min-w-0">
        <div className="flex items-center gap-3">
          <img src="/logo.svg" alt="Linggen" className="w-8 h-8" />
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
      </div>

      <div className="flex items-center gap-4 bg-slate-100 dark:bg-white/5 px-3 py-1.5 rounded-full border border-slate-200 dark:border-white/10 shadow-sm justify-self-end">
        <div className="flex items-center gap-2">
          <div className={cn('w-2 h-2 rounded-full', isRunning ? 'bg-green-500 animate-pulse' : 'bg-slate-400')} />
          <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">{isRunning ? 'Active' : 'Standby'}</span>
        </div>
        {onOpenMemory && (
          <>
            <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
            <button
              onClick={onOpenMemory}
              className="p-1 hover:text-blue-500 text-slate-500 transition-colors"
              title="Memory"
            >
              <Database size={14} />
            </button>
          </>
        )}
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
