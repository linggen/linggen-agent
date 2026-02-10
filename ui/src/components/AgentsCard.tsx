import React from 'react';
import { User, Bot } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, AgentWorkInfo, LeadState } from '../types';

export const AgentsCard: React.FC<{
  agents: AgentInfo[];
  leadState: LeadState | null;
  isRunning: boolean;
  selectedAgent: string;
  agentStatus?: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>;
  agentStatusText?: Record<string, string>;
  agentWork?: Record<string, AgentWorkInfo>;
}> = ({ agents, leadState, isRunning, selectedAgent, agentStatus, agentStatusText, agentWork }) => {
  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
      <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
        <h3 className="text-[10px] font-bold uppercase tracking-widest text-blue-500 flex items-center gap-2">
          <User size={12} /> Agents Status
        </h3>
        <span className="text-[10px] text-slate-400">Swarm</span>
      </div>
      <div className="flex-1 p-4 overflow-y-auto text-xs space-y-3">
        {agents.map((agent) => {
          const status = agentStatus?.[agent.name] ?? ((isRunning && selectedAgent === agent.name) ? 'thinking' : 'idle');
          const work = agentWork?.[agent.name];
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
                {agent.name === 'lead' ? <User size={14} className="text-blue-500" /> : <Bot size={14} className="text-purple-500" />}
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
            </div>
          </div>
        )})}

        <div className="pt-2 border-t border-slate-200 dark:border-white/5">
          <div className="text-[10px] text-slate-500 mb-1 font-bold uppercase tracking-widest">Active Lead Task</div>
          <div className="bg-slate-50 dark:bg-black/20 p-2.5 rounded-lg border border-slate-200 dark:border-white/5 italic text-[11px] text-slate-500 dark:text-slate-400 truncate">
            {leadState?.active_lead_task ? `${leadState.active_lead_task[1].substring(0, 100)}...` : 'No active task'}
          </div>
        </div>
      </div>
    </section>
  );
};
