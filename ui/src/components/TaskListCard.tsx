import React from 'react';
import type { Plan } from '../types';

const itemIcon: Record<string, string> = {
  pending: '○',
  in_progress: '◑',
  done: '●',
  skipped: '⊘',
};

const itemColor: Record<string, string> = {
  pending: 'text-slate-400',
  in_progress: 'text-blue-500',
  done: 'text-emerald-500',
  skipped: 'text-slate-300 dark:text-slate-600',
};

export const TaskListCard: React.FC<{ plan: Plan | null }> = ({ plan }) => {
  if (!plan) return null;
  // Only show executing or completed plans in the sidebar.
  // Plans with status 'planned' are shown inline in chat with approve/reject buttons.
  if (plan.status === 'planned' || plan.status === 'approved') return null;

  const doneCount = plan.items.filter(
    (i) => i.status === 'done' || i.status === 'skipped'
  ).length;
  const totalCount = plan.items.length;

  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-slate-100 dark:border-white/5">
        <span className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400">
          {plan.origin === 'user_requested' ? 'Plan' : 'Tasks'}
        </span>
        <span className="text-[10px] font-semibold tabular-nums text-slate-400 dark:text-slate-500">
          {doneCount}/{totalCount}
        </span>
      </div>

      {/* Summary */}
      {plan.summary && (
        <div className="px-3 pt-2 text-[11px] text-slate-500 dark:text-slate-400 leading-snug">
          {plan.summary}
        </div>
      )}

      {/* Items */}
      <div className="px-3 py-2 space-y-1">
        {plan.items.map((item, idx) => (
          <div key={idx} className="flex items-start gap-1.5 text-[11px] leading-relaxed">
            <span
              className={`font-mono shrink-0 ${itemColor[item.status] || 'text-slate-400'}${
                item.status === 'in_progress' ? ' animate-pulse' : ''
              }`}
            >
              {itemIcon[item.status] || '○'}
            </span>
            <span
              className={
                item.status === 'done'
                  ? 'line-through opacity-50'
                  : item.status === 'skipped'
                    ? 'line-through opacity-40'
                    : ''
              }
            >
              {item.title}
            </span>
          </div>
        ))}
      </div>

      {/* Completed badge */}
      {plan.status === 'completed' && (
        <div className="px-3 pb-2">
          <span className="text-[9px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400">
            Completed
          </span>
        </div>
      )}
    </section>
  );
};
