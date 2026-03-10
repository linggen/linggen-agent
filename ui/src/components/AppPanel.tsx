import React from 'react';
import { X } from 'lucide-react';

export interface AppPanelState {
  skill: string;
  launcher: string;
  url: string;
  title: string;
  width?: number;
  height?: number;
}

export const AppPanel: React.FC<{
  app: AppPanelState;
  onClose: () => void;
}> = ({ app, onClose }) => {
  const width = app.width || 800;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
      <div
        className="bg-white dark:bg-zinc-900 rounded-2xl shadow-2xl border border-slate-200 dark:border-white/10 overflow-hidden flex flex-col"
        style={{ width: Math.min(width + 32, window.innerWidth - 40), maxHeight: '90vh' }}
      >
        <div className="flex items-center justify-between px-4 py-2.5 border-b border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-zinc-800/50 shrink-0">
          <div className="flex items-center gap-2">
            <span className="text-xs font-semibold text-slate-700 dark:text-slate-200">{app.title}</span>
            <span className="text-[9px] font-mono text-slate-400 dark:text-slate-500">{app.skill}</span>
          </div>
          <button
            onClick={onClose}
            className="p-1 hover:bg-slate-200 dark:hover:bg-white/10 rounded-lg transition-colors text-slate-400 hover:text-slate-600 dark:hover:text-slate-300"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-hidden">
          <iframe
            src={app.url}
            title={app.title}
            style={{ width: '100%', height: '100%', border: 'none' }}
            sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
          />
        </div>
      </div>
    </div>
  );
};
