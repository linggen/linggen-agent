import React from 'react';
import { X } from 'lucide-react';
import { cn } from '../../lib/cn';
import type {
  ChatMessage,
  SubagentInfo,
} from '../../types';
import { MarkdownContent } from './MarkdownContent';
import { visibleMessageText, statusBadgeClass } from './MessageHelpers';

export const SubagentDrawer: React.FC<{
  selectedSubagent: SubagentInfo;
  filteredSubagentMessages: ChatMessage[];
  subagentMessageFilter: string;
  setSubagentMessageFilter: (value: string) => void;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onClose: () => void;
}> = ({
  selectedSubagent,
  filteredSubagentMessages,
  subagentMessageFilter,
  setSubagentMessageFilter,
  cancellingRunIds,
  onCancelRun,
  onClose,
}) => (
  <div className="absolute inset-y-0 right-0 w-[min(26rem,95%)] bg-white dark:bg-[#0f0f0f] border-l border-slate-200 dark:border-white/10 shadow-2xl z-[65] flex flex-col">
    <div className="px-4 py-3 border-b border-slate-200 dark:border-white/10 flex items-start justify-between gap-3">
      <div>
        <div className="text-xs font-bold uppercase tracking-wider text-slate-500">Subagent Context</div>
        <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{selectedSubagent.id}</div>
        <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">
          {selectedSubagent.folder}/{selectedSubagent.file}
        </div>
      </div>
      <div className="flex items-center gap-2">
        <span className={cn('text-[11px] px-2 py-1 rounded-full uppercase tracking-wide', statusBadgeClass(selectedSubagent.status))}>
          {selectedSubagent.status}
        </span>
        {selectedSubagent.status === 'running' && onCancelRun && (
          <button
            onClick={() => onCancelRun(selectedSubagent.id)}
            disabled={!!cancellingRunIds?.[selectedSubagent.id]}
            className={cn(
              'px-2 py-1 rounded-lg text-[11px] font-semibold border transition-colors',
              cancellingRunIds?.[selectedSubagent.id]
                ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
            )}
          >
            {cancellingRunIds?.[selectedSubagent.id] ? 'Cancelling...' : 'Cancel Run'}
          </button>
        )}
        <button
          onClick={onClose}
          className="p-1.5 rounded-lg hover:bg-slate-100 dark:hover:bg-white/10 text-slate-500"
          title="Close"
        >
          <X size={14} />
        </button>
      </div>
    </div>

    <div className="p-4 border-b border-slate-200 dark:border-white/10">
      <div className="text-[11px] uppercase tracking-widest text-slate-500 mb-2">Active Paths</div>
      <div className="space-y-1 max-h-28 overflow-auto custom-scrollbar">
        {selectedSubagent.paths.map((path) => (
          <div key={path} className="text-[12px] font-mono text-slate-600 dark:text-slate-300 truncate">
            {path}
          </div>
        ))}
      </div>
    </div>

    <div className="flex-1 overflow-auto p-4 space-y-3 custom-scrollbar">
      <div className="flex items-center justify-between gap-2">
        <div className="text-[11px] uppercase tracking-widest text-slate-500">Messages</div>
        <input
          value={subagentMessageFilter}
          onChange={(e) => setSubagentMessageFilter(e.target.value)}
          placeholder="Filter messages"
          className="w-40 text-[12px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
        />
      </div>
      {filteredSubagentMessages.length === 0 && (
        <div className="text-xs italic text-slate-500">No context messages captured for this subagent yet.</div>
      )}
      {filteredSubagentMessages.slice(-20).map((msg, idx) => (
        <div key={`${msg.timestamp}-${idx}-${msg.from || msg.role}`} className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 p-2.5">
          <div className="text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            {(msg.from || msg.role).toUpperCase()} {msg.to ? `\u2192 ${msg.to.toUpperCase()}` : ''}
          </div>
          <div className="text-[13px] text-slate-700 dark:text-slate-200 break-words">
            <MarkdownContent text={visibleMessageText(msg)} />
          </div>
        </div>
      ))}
    </div>
  </div>
);
