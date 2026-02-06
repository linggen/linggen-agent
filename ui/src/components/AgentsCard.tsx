import React from 'react';
import { User, Bot } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, LeadState } from '../types';

export const AgentsCard: React.FC<{
  agents: AgentInfo[];
  leadState: LeadState | null;
  isRunning: boolean;
  selectedAgent: string;
  agentStatus?: Record<string, 'idle' | 'working'>;
}> = ({ agents, leadState, isRunning, selectedAgent, agentStatus }) => {
  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
      <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
        <h3 className="text-[10px] font-bold uppercase tracking-widest text-blue-500 flex items-center gap-2">
          <User size={12} /> Agents Status
        </h3>
        <span className="text-[10px] text-slate-400">Swarm</span>
      </div>
      <div className="flex-1 p-4 overflow-y-auto text-xs space-y-4">
        {agents.map((agent) => {
          const status = agentStatus?.[agent.name] ?? ((isRunning && selectedAgent === agent.name) ? 'working' : 'idle');
          return (
          <div
            key={agent.name}
            className="flex items-center justify-between bg-slate-50 dark:bg-black/20 px-3 py-2 rounded-lg border border-slate-200 dark:border-white/5"
          >
            <div className="flex items-center gap-2">
              {agent.name === 'lead' ? <User size={14} className="text-blue-500" /> : <Bot size={14} className="text-purple-500" />}
              <span className="font-bold uppercase tracking-tight">{agent.name}</span>
            </div>
            <div className="flex items-center gap-2">
              <span
                className={cn(
                  'text-[8px] font-bold px-1.5 py-0.5 rounded-full uppercase',
                  status === 'working'
                    ? 'bg-green-500/20 text-green-500 animate-pulse'
                    : 'bg-slate-500/20 text-slate-500'
                )}
              >
                {status === 'working' ? 'Working' : 'Idle'}
              </span>
            </div>
          </div>
        )})}

        <div className="pt-2 border-t border-slate-200 dark:border-white/5">
          <div className="text-[10px] text-slate-500 mb-1 font-bold uppercase tracking-widest">Active Lead Task</div>
          <div className="bg-slate-50 dark:bg-black/20 p-2 rounded border border-slate-200 dark:border-white/5 italic text-[10px] text-slate-400 truncate">
            {leadState?.active_lead_task ? `${leadState.active_lead_task[1].substring(0, 100)}...` : 'No active task'}
          </div>
        </div>
      </div>
    </section>
  );
};
