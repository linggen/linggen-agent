import React from 'react';
import { Activity, FileText, FolderOpen } from 'lucide-react';
import type { AgentTreeItem } from '../types';

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const splitPath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) return { folder: '.', file: path };
  return { folder: path.slice(0, idx) || '.', file: path.slice(idx + 1) };
};

const collectEntries = (
  nodeMap: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[]
) => {
  if (!nodeMap) return;
  Object.values(nodeMap).forEach((item) => {
    if (item.type === 'file') {
      if (!item.path || !item.agent) return;
      out.push({
        path: item.path,
        agent: item.agent,
        status: item.status || 'idle',
        lastModified: Number(item.last_modified || 0),
      });
      return;
    }
    collectEntries(item.children, out);
  });
};

export const AgentTree: React.FC<{ agentTree: Record<string, AgentTreeItem>; onSelect: (path: string) => void }> = ({
  agentTree,
  onSelect,
}) => {
  const entries: ActivityEntry[] = [];
  collectEntries(agentTree, entries);

  const working = entries
    .filter((entry) => entry.status === 'working')
    .sort((a, b) => b.lastModified - a.lastModified);
  const visible = working;
  const groups = visible.reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
    if (!acc[entry.agent]) acc[entry.agent] = [];
    acc[entry.agent].push(entry);
    return acc;
  }, {});

  const repoName = Object.keys(agentTree)[0] || 'repo';

  return (
    <div className="flex-1 overflow-y-auto p-2">
      <div className="px-2 py-1.5 text-[10px] uppercase tracking-wider text-slate-500 dark:text-slate-400">
        {repoName}
      </div>
      {visible.length === 0 && (
        <div className="p-4 text-xs text-slate-500 italic text-center">
          No active paths yet. Paths will appear when agents start working.
        </div>
      )}
      {Object.entries(groups).map(([agent, agentEntries]) => (
        <section
          key={agent}
          className="mb-2 rounded-xl border border-slate-200 dark:border-white/5 bg-white dark:bg-black/20 overflow-hidden"
        >
          <div className="px-3 py-2 text-[10px] font-bold uppercase tracking-widest flex items-center justify-between border-b border-slate-200 dark:border-white/5">
            <span className="text-slate-600 dark:text-slate-300">{agent}</span>
            <span className="text-[9px] text-blue-500 flex items-center gap-1">
              <Activity size={10} />
              active
            </span>
          </div>
          <div className="p-1.5 space-y-1">
            {agentEntries.map((entry) => {
              const parts = splitPath(entry.path);
              return (
                <button
                  key={`${agent}:${entry.path}`}
                  onClick={() => onSelect(entry.path)}
                  className="w-full text-left px-2 py-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-slate-800 dark:text-slate-100 flex items-center gap-1.5">
                        <FileText size={12} className="shrink-0 text-blue-500" />
                        <span className="truncate">{parts.file}</span>
                      </div>
                      <div className="text-[10px] text-slate-500 dark:text-slate-400 truncate flex items-center gap-1.5 mt-0.5">
                        <FolderOpen size={11} className="shrink-0" />
                        <span>{parts.folder}</span>
                      </div>
                    </div>
                    <span
                      className={`text-[9px] px-1.5 py-0.5 rounded-full uppercase tracking-wide shrink-0 ${
                        entry.status === 'working'
                          ? 'bg-blue-500/20 text-blue-600 dark:text-blue-400'
                          : 'bg-slate-500/20 text-slate-500'
                      }`}
                    >
                      active
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
};
