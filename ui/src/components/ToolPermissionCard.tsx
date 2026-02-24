import React from 'react';
import type { PendingAskUser, AskUserAnswer } from '../types';

interface ToolPermissionCardProps {
  pending: PendingAskUser;
  onRespond: (questionId: string, answers: AskUserAnswer[]) => void;
}

export const ToolPermissionCard: React.FC<ToolPermissionCardProps> = ({ pending, onRespond }) => {
  const question = pending.questions[0];
  if (!question) return null;

  const handleClick = (label: string) => {
    onRespond(pending.questionId, [
      { question_index: 0, selected: [label], custom_text: null },
    ]);
  };

  // Determine if the option is "Cancel" to style it differently.
  const isCancel = (label: string) => label === 'Cancel';

  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-amber-200 dark:border-amber-500/20 shadow-sm flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-amber-100 dark:border-amber-500/10 bg-amber-50/50 dark:bg-amber-500/5">
        <svg className="w-3.5 h-3.5 text-amber-500 dark:text-amber-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v2m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        <span className="text-[11px] font-bold uppercase tracking-wider text-amber-600 dark:text-amber-400">
          Permission Required
        </span>
      </div>

      {/* Question */}
      <div className="px-3 py-3 space-y-3">
        <p className="text-[12px] text-slate-700 dark:text-slate-300 font-mono leading-snug">
          {question.question}
        </p>

        {/* Vertical button list */}
        <div className="flex flex-col gap-1.5">
          {question.options.map((opt) => (
            <button
              key={opt.label}
              onClick={() => handleClick(opt.label)}
              className={`text-left text-[11px] px-3 py-1.5 rounded-lg border transition-colors cursor-pointer ${
                isCancel(opt.label)
                  ? 'bg-white text-slate-400 border-slate-200 hover:border-red-300 hover:text-red-500 dark:bg-white/5 dark:text-slate-500 dark:border-white/10 dark:hover:border-red-500/40 dark:hover:text-red-400'
                  : 'bg-white text-slate-600 border-slate-200 hover:border-amber-300 hover:text-amber-700 hover:bg-amber-50/50 dark:bg-white/5 dark:text-slate-300 dark:border-white/10 dark:hover:border-amber-500/40 dark:hover:bg-amber-500/5'
              }`}
            >
              <span className="font-medium">{opt.label}</span>
              {opt.description && (
                <span className="ml-2 text-[10px] text-slate-400 dark:text-slate-500">
                  {opt.description}
                </span>
              )}
            </button>
          ))}
        </div>
      </div>
    </section>
  );
};
