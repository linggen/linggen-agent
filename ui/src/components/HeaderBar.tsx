import React from 'react';
import { Sparkles, RefreshCw, Folder, Trash2 } from 'lucide-react';
import { cn } from '../lib/cn';
import type { ProjectInfo } from '../types';

export const HeaderBar: React.FC<{
  projects: ProjectInfo[];
  selectedProjectRoot: string;
  setSelectedProjectRoot: (value: string) => void;
  showAddProject: boolean;
  setShowAddProject: (value: boolean) => void;
  newProjectPath: string;
  setNewProjectPath: (value: string) => void;
  addProject: () => void;
  removeProject: (path: string) => void;
  pickFolder: () => void;
  refreshPageState: () => void;
  isRunning: boolean;
  currentMode: 'chat' | 'auto';
  onModeChange: (mode: 'chat' | 'auto') => void;
}> = ({
  projects,
  selectedProjectRoot,
  setSelectedProjectRoot,
  showAddProject,
  setShowAddProject,
  newProjectPath,
  setNewProjectPath,
  addProject,
  removeProject,
  pickFolder,
  refreshPageState,
  isRunning,
  currentMode,
  onModeChange,
}) => {
  return (
    <header className="flex items-center justify-between px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
      <div className="flex items-center gap-6">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 bg-blue-600 rounded-lg flex items-center justify-center shadow-lg shadow-blue-600/20">
            <Sparkles size={18} className="text-white" />
          </div>
          <h1 className="text-lg font-bold tracking-tight text-slate-900 dark:text-white">Linggen Agent</h1>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={refreshPageState}
            disabled={!selectedProjectRoot || isRunning}
            className="p-1.5 hover:bg-blue-500/10 hover:text-blue-500 rounded-lg text-slate-500 transition-colors disabled:opacity-50"
            title="Refresh page state"
          >
            <RefreshCw size={16} className={cn(isRunning && 'animate-spin')} />
          </button>
          <select
            value={selectedProjectRoot}
            onChange={(e) => setSelectedProjectRoot(e.target.value)}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-1.5 outline-none font-mono max-w-[360px]"
          >
            {projects.map((p) => (
              <option key={p.path} value={p.path}>
                {p.name} ({p.path})
              </option>
            ))}
          </select>
          <button
            onClick={() => setShowAddProject(!showAddProject)}
            className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-slate-500"
            title="Manage Projects"
          >
            <Folder size={16} />
          </button>
          {selectedProjectRoot && (
            <button
              onClick={() => removeProject(selectedProjectRoot)}
              className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-lg text-slate-500 transition-colors"
              title="Remove Current Project"
            >
              <Trash2 size={16} />
            </button>
          )}
        </div>
      </div>

      {showAddProject && (
        <div className="absolute top-14 left-1/2 -translate-x-1/2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-xl p-4 shadow-2xl z-[60] flex flex-col gap-3 w-[min(42rem,90vw)]">
          <div className="flex gap-2">
            <input
              value={newProjectPath}
              onChange={(e) => setNewProjectPath(e.target.value)}
              placeholder="Full path to repository..."
              className="flex-1 bg-slate-100 dark:bg-white/5 border-none rounded-lg px-3 py-2 text-xs outline-none"
            />
            <button
              onClick={pickFolder}
              className="px-3 py-2 bg-slate-200 dark:bg-white/10 rounded-lg text-[10px] font-bold hover:bg-slate-300 dark:hover:bg-white/20 transition-colors"
              title="Browse..."
            >
              Browse
            </button>
          </div>
          <button
            onClick={addProject}
            className="w-full py-2 bg-blue-600 text-white rounded-lg text-[10px] font-bold shadow-lg shadow-blue-600/20"
          >
            Add Project
          </button>
        </div>
      )}

      <div className="flex items-center gap-4 bg-slate-100 dark:bg-white/5 px-3 py-1.5 rounded-full border border-slate-200 dark:border-white/10 shadow-sm">
        <div className="flex items-center gap-2">
          <div className={cn('w-2 h-2 rounded-full', isRunning ? 'bg-green-500 animate-pulse' : 'bg-slate-400')} />
          <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">{isRunning ? 'Active' : 'Standby'}</span>
        </div>
        <div className="w-px h-3 bg-slate-300 dark:bg-white/10" />
        <select
          value={currentMode}
          onChange={(e) => onModeChange((e.target.value === 'chat' ? 'chat' : 'auto'))}
          className="text-[10px] font-bold text-blue-600 dark:text-blue-400 uppercase tracking-widest bg-transparent outline-none"
          title="Prompt mode"
        >
          <option value="auto">Mode: Auto</option>
          <option value="chat">Mode: Chat</option>
        </select>
      </div>
    </header>
  );
};
