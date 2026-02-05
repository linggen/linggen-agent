import React, { useState } from 'react';
import { FileText, Folder, ChevronRight, ChevronDown } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentTreeItem } from '../types';

const TreeNode: React.FC<{ name: string; item: AgentTreeItem; depth?: number; onSelect: (path: string) => void }> = ({ name, item, depth = 0, onSelect }) => {
  const [isOpen, setIsOpen] = useState(true);

  if (item.type === 'file') {
    return (
      <button
        onClick={() => onSelect(item.path || name)}
        className="w-full flex items-center justify-between px-2 py-1 rounded text-xs hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        <div className="flex items-center gap-2 truncate">
          <FileText size={12} className="text-slate-400 shrink-0" />
          <span className="truncate">{name}</span>
        </div>
        {item.agent && (
          <span
            className={cn(
              "text-[8px] font-bold px-1.5 py-0.5 rounded-full uppercase tracking-tighter shrink-0 ml-2",
              item.status === 'working' ? 'bg-blue-500/20 text-blue-500' : 'bg-slate-500/20 text-slate-500'
            )}
          >
            {item.agent} {item.status === 'working' ? '...' : ''}
          </span>
        )}
      </button>
    );
  }

  return (
    <div>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="w-full flex items-center gap-2 px-2 py-1 rounded text-xs hover:bg-slate-100 dark:hover:bg-white/5 transition-colors text-slate-500"
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
      >
        {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <Folder size={12} className="text-blue-400 shrink-0" />
        <span className="truncate font-bold">{name}</span>
      </button>
      {isOpen && item.children && (
        <div className="flex flex-col">
          {Object.entries(item.children).map(([childName, childItem]) => (
            <TreeNode key={childName} name={childName} item={childItem} depth={depth + 1} onSelect={onSelect} />
          ))}
        </div>
      )}
    </div>
  );
};

export const AgentTree: React.FC<{ agentTree: Record<string, AgentTreeItem>; onSelect: (path: string) => void }> = ({
  agentTree,
  onSelect,
}) => {
  return (
    <div className="flex-1 overflow-y-auto p-2">
      {Object.entries(agentTree).length === 0 && (
        <div className="p-4 text-xs text-slate-500 italic text-center">
          No files tracked yet. Agents will appear here as they work.
        </div>
      )}
      {Object.entries(agentTree).map(([name, item]) => (
        <TreeNode key={name} name={name} item={item} onSelect={onSelect} />
      ))}
    </div>
  );
};
