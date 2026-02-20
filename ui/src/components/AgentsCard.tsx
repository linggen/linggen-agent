import React from 'react';
import { Bot } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, AgentRunSummary, AgentWorkInfo, WorkspaceState } from '../types';

const formatTime = (ts?: number | null) => {
  if (!ts || ts <= 0) return '-';
  return new Date(ts * 1000).toLocaleTimeString();
};

const contextPercent = (tokens: number, tokenLimit?: number) => {
  const limit = tokenLimit && tokenLimit > 0 ? tokenLimit : undefined;
  if (!limit) return null;
  if (!tokens || tokens <= 0) return 0;
  return Math.round((tokens / limit) * 100);
};

const formatCompactInt = (n: number) => {
  if (!Number.isFinite(n)) return '';
  if (n >= 1_000_000) return `${Math.round(n / 100_000) / 10}m`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${Math.round(n / 100) / 10}k`;
  return `${n}`;
};

export const AgentsCard: React.FC<{
  agents: AgentInfo[];
  workspaceState: WorkspaceState | null;
  isRunning: boolean;
  selectedAgent: string;
  agentStatus?: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>;
  agentStatusText?: Record<string, string>;
  agentWork?: Record<string, AgentWorkInfo>;
  agentRunSummary?: Record<string, AgentRunSummary>;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
}> = ({ agents, workspaceState, isRunning, selectedAgent, agentStatus, agentStatusText, agentWork, agentRunSummary, agentContext }) => {
  return (
      <div className="flex-1 p-4 overflow-y-auto text-xs space-y-3">
        {agents.map((agent) => {
          const status = agentStatus?.[agent.name] ?? ((isRunning && selectedAgent === agent.name) ? 'thinking' : 'idle');
          const work = agentWork?.[agent.name];
          const run = agentRunSummary?.[agent.name.toLowerCase()];
          const context = agentContext?.[agent.name.toLowerCase()];
          const contextTokens = Number(context?.tokens || 0);
          const contextLimit =
            context && typeof context.tokenLimit === 'number' && context.tokenLimit > 0
              ? Number(context.tokenLimit)
              : null;
          const contextPct = contextPercent(contextTokens, contextLimit ?? undefined);
          const statusText =
            agentStatusText?.[agent.name] ||
            (status === 'calling_tool'
              ? 'Calling Tool'
              : status === 'model_loading'
                ? 'Model Loading'
              : status === 'thinking'
                ? 'Thinking'
                : status === 'working'
                  ? 'Working'
                  : 'Idle');
          return (
          <div
            key={agent.name}
            className="bg-slate-50 dark:bg-black/20 px-3 py-2.5 rounded-xl border border-slate-200 dark:border-white/5"
          >
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Bot size={14} className="text-purple-500" />
                <span className="font-bold uppercase tracking-tight">{agent.name}</span>
              </div>
              <div className="flex items-center gap-2">
                <span
                  className={cn(
                    'text-[9px] font-bold px-2 py-0.5 rounded-full uppercase tracking-wide',
                    status === 'calling_tool'
                      ? 'bg-amber-500/20 text-amber-600 animate-pulse'
                      : status === 'model_loading'
                      ? 'bg-indigo-500/20 text-indigo-600 animate-pulse'
                      : status === 'thinking'
                      ? 'bg-blue-500/20 text-blue-500 animate-pulse'
                      : status === 'working'
                        ? 'bg-green-500/20 text-green-500 animate-pulse'
                        : 'bg-slate-500/20 text-slate-500'
                  )}
                >
                  {statusText}
                </span>
              </div>
            </div>
            <div className="mt-2 text-[10px] text-slate-500 dark:text-slate-400 leading-relaxed">
              <div className="truncate">
                <span className="font-semibold text-slate-600 dark:text-slate-300">Folder:</span>{' '}
                {work?.folder || '-'}
              </div>
              <div className="truncate">
                <span className="font-semibold text-slate-600 dark:text-slate-300">File:</span>{' '}
                {work?.file || '-'}
                {work && work.activeCount > 1 ? ` (+${work.activeCount - 1})` : ''}
              </div>
              <div
                className="mt-1 flex items-center gap-2 text-blue-500/80 font-medium"
                title={
                  context
                    ? `${contextTokens.toLocaleString()} tokens • ${context.messages} msgs${
                        contextLimit ? ` (model max ctx ~${contextLimit.toLocaleString()} tokens)` : ''
                      }`
                    : 'Context telemetry not available yet'
                }
              >
                <span className="flex items-center gap-1">
                  <span className="font-semibold">Context:</span>
                  {context
                    ? `${formatCompactInt(contextTokens)} / ${
                        contextLimit ? formatCompactInt(contextLimit) : 'n/a'
                      } (${contextPct !== null ? `${contextPct}%` : 'n/a'})`
                    : 'n/a'}
                </span>
              </div>
              {run && (
                <div className="mt-2 flex flex-wrap items-center gap-1.5">
                  <span
                    className={cn(
                      'text-[9px] px-1.5 py-0.5 rounded-full uppercase tracking-wide font-semibold',
                      run.status === 'running'
                        ? 'bg-green-500/20 text-green-600 dark:text-green-400'
                        : run.status === 'failed'
                          ? 'bg-red-500/20 text-red-600 dark:text-red-400'
                          : run.status === 'cancelled'
                            ? 'bg-amber-500/20 text-amber-600 dark:text-amber-400'
                            : 'bg-slate-500/20 text-slate-600 dark:text-slate-300'
                    )}
                  >
                    run {run.status}
                  </span>
                  <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-blue-500/10 text-blue-600 dark:text-blue-400 font-semibold">
                    timeline {run.timeline_events}
                  </span>
                  <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-purple-500/10 text-purple-600 dark:text-purple-400 font-semibold">
                    sub {run.child_count}
                  </span>
                  <span className="text-[9px] text-slate-500 dark:text-slate-400">
                    @{formatTime(run.last_event_at)}
                  </span>
                </div>
              )}
            </div>
          </div>
        )})}

        <div className="pt-2 border-t border-slate-200 dark:border-white/5">
          <div className="text-[10px] text-slate-500 mb-1 font-bold uppercase tracking-widest">Active Task</div>
          <div className="bg-slate-50 dark:bg-black/20 p-2.5 rounded-lg border border-slate-200 dark:border-white/5 italic text-[11px] text-slate-500 dark:text-slate-400 truncate">
            {workspaceState?.active_task ? `${workspaceState.active_task[1].substring(0, 100)}...` : 'No active task'}
          </div>
        </div>
      </div>
  );
};
