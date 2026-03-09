import React, { useState, useEffect, useCallback } from 'react';
import { ArrowLeft, Target, Plus, Play, Pause, Trash2, Clock, History, Edit3, Check, X, Eye, ExternalLink, FolderOpen } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, CronMission, MissionRunEntry, MissionTab, ProjectInfo } from '../types';

const formatTimestamp = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
};

const timeSince = (ts: number) => {
  if (!ts || ts <= 0) return '';
  const now = Date.now();
  const diffMs = now - ts * 1000;
  if (diffMs < 0) return '';
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
};

/** Human-readable description of a cron schedule. */
const describeCron = (schedule: string): string => {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;

  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
    return `Every ${min.slice(2)} minutes`;
  }
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') {
    return `Every ${hour.slice(2)} hours at minute ${min}`;
  }
  if (dom === '*' && mon === '*' && dow === '*') {
    return `Daily at ${hour}:${min.padStart(2, '0')}`;
  }
  if (dom === '*' && mon === '*' && dow !== '*') {
    const dayNames: Record<string, string> = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun' };
    const days = dow.split(',').map(d => dayNames[d] || d).join(', ');
    if (dow.includes('-')) {
      const [start, end] = dow.split('-');
      return `${dayNames[start] || start}-${dayNames[end] || end} at ${hour}:${min.padStart(2, '0')}`;
    }
    return `${days} at ${hour}:${min.padStart(2, '0')}`;
  }
  return schedule;
};

const projectLabel = (path: string | null | undefined, projects: ProjectInfo[]): string | null => {
  if (!path) return null;
  const proj = projects.find(p => p.path === path);
  if (proj) return proj.name || path.split('/').pop() || path;
  return path.split('/').pop() || path;
};

// --- API helpers (global, no project_root required) ---

async function fetchMissions(): Promise<CronMission[]> {
  const resp = await fetch('/api/missions');
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.missions) ? data.missions : [];
}

async function createMission(name: string | undefined, schedule: string, prompt: string, model?: string, project?: string): Promise<CronMission | null> {
  const resp = await fetch('/api/missions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: name || null, schedule, prompt, model: model || null, project: project || null }),
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text);
  }
  return resp.json();
}

async function updateMission(id: string, updates: Record<string, any>): Promise<CronMission | null> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(updates),
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text);
  }
  return resp.json();
}

async function deleteMission(id: string): Promise<void> {
  await fetch(`/api/missions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

async function fetchMissionRuns(id: string): Promise<MissionRunEntry[]> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/runs`);
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.runs) ? data.runs : [];
}

async function triggerMission(id: string, project?: string): Promise<void> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/trigger`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_root: project || null }),
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text);
  }
}

// --- Mission List ---

const MissionCard: React.FC<{
  mission: CronMission;
  projects: ProjectInfo[];
  onToggle: (id: string, enabled: boolean) => void;
  onEdit: (m: CronMission) => void;
  onDelete: (id: string) => void;
  onViewRuns: (m: CronMission) => void;
  onTrigger: (m: CronMission) => void;
}> = ({ mission, projects, onToggle, onEdit, onDelete, onViewRuns, onTrigger }) => {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const projLabel = projectLabel(mission.project, projects);

  return (
    <div className={cn(
      'border rounded-lg p-4 bg-white dark:bg-white/[0.02] transition-colors',
      mission.enabled
        ? 'border-green-500/20'
        : 'border-slate-200 dark:border-white/10 opacity-60',
    )}>
      <div className="flex items-start justify-between gap-3 mb-2">
        <div className="flex items-center gap-2 min-w-0">
          <button
            onClick={() => onToggle(mission.id, !mission.enabled)}
            className={cn(
              'w-8 h-5 rounded-full relative transition-colors shrink-0',
              mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
            )}
          >
            <span className={cn(
              'absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform',
              mission.enabled ? 'left-3.5' : 'left-0.5',
            )} />
          </button>
          <span className="text-xs font-mono text-blue-600 dark:text-blue-400 bg-blue-500/10 px-2 py-0.5 rounded">
            {mission.schedule}
          </span>
          <span className="text-[10px] text-slate-500 truncate">{describeCron(mission.schedule)}</span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => onTrigger(mission)}
            className="p-1 rounded hover:bg-green-100 dark:hover:bg-green-500/10 text-slate-400 hover:text-green-600"
            title="Run now"
          >
            <Play size={14} />
          </button>
          <button
            onClick={() => onViewRuns(mission)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600"
            title="View runs"
          >
            <History size={14} />
          </button>
          <button
            onClick={() => onEdit(mission)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600"
            title="Edit"
          >
            <Edit3 size={14} />
          </button>
          {confirmDelete ? (
            <div className="flex items-center gap-0.5">
              <button
                onClick={() => { onDelete(mission.id); setConfirmDelete(false); }}
                className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-red-500"
                title="Confirm delete"
              >
                <Check size={14} />
              </button>
              <button
                onClick={() => setConfirmDelete(false)}
                className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400"
                title="Cancel"
              >
                <X size={14} />
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfirmDelete(true)}
              className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500"
              title="Delete"
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
      </div>

      {mission.name && (
        <div className="text-sm font-medium text-slate-800 dark:text-slate-200 mb-1.5">
          {mission.name}
        </div>
      )}

      <div className="flex items-center gap-2 mb-2">
        <span className="text-[10px] font-bold uppercase tracking-wide px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-600 dark:text-purple-400">
          mission
        </span>
        {mission.model && (
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-slate-100 dark:bg-white/5 text-slate-500">
            {mission.model}
          </span>
        )}
        {projLabel ? (
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-600 dark:text-blue-400 flex items-center gap-1">
            <FolderOpen size={9} /> {projLabel}
          </span>
        ) : (
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-slate-100 dark:bg-white/5 text-slate-400 italic">
            global
          </span>
        )}
        <span className="text-[10px] text-slate-400 ml-auto">
          Created {timeSince(mission.created_at)}
        </span>
      </div>

      <div className="text-xs text-slate-700 dark:text-slate-300 line-clamp-2 whitespace-pre-wrap">
        {mission.prompt}
      </div>
    </div>
  );
};

const MissionList: React.FC<{
  missions: CronMission[];
  projects: ProjectInfo[];
  onToggle: (id: string, enabled: boolean) => void;
  onEdit: (m: CronMission) => void;
  onDelete: (id: string) => void;
  onViewRuns: (m: CronMission) => void;
  onTrigger: (m: CronMission) => void;
  onCreate: () => void;
}> = ({ missions, projects, onToggle, onEdit, onDelete, onViewRuns, onTrigger, onCreate }) => {
  const enabledCount = missions.filter(m => m.enabled).length;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
            Missions ({missions.length})
          </h2>
          {enabledCount > 0 && (
            <span className="text-[10px] font-bold px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
              {enabledCount} active
            </span>
          )}
        </div>
        <button
          onClick={onCreate}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors"
        >
          <Plus size={14} /> New Mission
        </button>
      </div>

      {missions.length === 0 ? (
        <div className="text-center py-16">
          <Target size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
          <p className="text-sm text-slate-500">No missions yet</p>
          <p className="text-[11px] text-slate-400 mt-1">Create a mission to schedule agent tasks</p>
        </div>
      ) : (
        <div className="space-y-3">
          {missions.map(m => (
            <MissionCard
              key={m.id}
              mission={m}
              projects={projects}
              onToggle={onToggle}
              onEdit={onEdit}
              onDelete={onDelete}
              onViewRuns={onViewRuns}
              onTrigger={onTrigger}
            />
          ))}
        </div>
      )}
    </div>
  );
};

// --- Mission Editor (Create/Edit) ---

const CRON_PRESETS = [
  { label: 'Every 30 min', value: '*/30 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 2 hours', value: '0 */2 * * *' },
  { label: 'Daily at 9am', value: '0 9 * * *' },
  { label: 'Weekdays 9am', value: '0 9 * * 1-5' },
  { label: 'Weekly Sunday', value: '0 0 * * 0' },
];

const MissionEditor: React.FC<{
  editing: CronMission | null;
  agents: AgentInfo[];
  projects: ProjectInfo[];
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
  onViewAgent: () => void;
}> = ({ editing, agents: _agents, projects, onSave, onCancel, onViewAgent }) => {
  const [name, setName] = useState(editing?.name || '');
  const [schedule, setSchedule] = useState(editing?.schedule || '*/30 * * * *');
  const [prompt, setPrompt] = useState(editing?.prompt || '');
  const [model, setModel] = useState(editing?.model || '');
  const [selectedProject, setSelectedProject] = useState(editing?.project || '');
  const [models, setModels] = useState<{ id: string; model: string; provider: string }[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetch('/api/config')
      .then(r => r.ok ? r.json() : null)
      .then(data => {
        if (data?.models) setModels(data.models);
      })
      .catch(() => {});
  }, []);

  const handleSave = async () => {
    if (!schedule.trim() || !prompt.trim()) {
      setError('Schedule and prompt are required');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      let result: CronMission | null;
      if (editing) {
        result = await updateMission(editing.id, {
          name: name || null,
          schedule,
          prompt,
          model: model || null,
          project: selectedProject || null,
        });
      } else {
        result = await createMission(name || undefined, schedule, prompt, model || undefined, selectedProject || undefined);
      }
      if (result) onSave(result);
    } catch (e: any) {
      setError(e.message || 'Failed to save mission');
    }
    setSaving(false);
  };

  return (
    <div className="space-y-4">
      <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
        {editing ? 'Edit Mission' : 'New Mission'}
      </h2>

      {error && (
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-3 text-xs text-red-600 dark:text-red-400">
          {error}
        </div>
      )}

      {/* Name */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Name
        </label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Daily code review"
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
      </div>

      {/* Schedule */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Cron Schedule
        </label>
        <input
          type="text"
          value={schedule}
          onChange={(e) => setSchedule(e.target.value)}
          placeholder="*/30 * * * *"
          className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
        <div className="flex flex-wrap gap-1.5 mt-2">
          {CRON_PRESETS.map(p => (
            <button
              key={p.value}
              onClick={() => setSchedule(p.value)}
              className={cn(
                'text-[10px] px-2 py-0.5 rounded-full border transition-colors',
                schedule === p.value
                  ? 'border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400'
                  : 'border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5',
              )}
            >
              {p.label}
            </button>
          ))}
        </div>
        <div className="text-[10px] text-slate-400 mt-1.5">
          {describeCron(schedule)}
        </div>
      </div>

      {/* Agent (readonly) */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Agent
        </label>
        <div className="flex items-center gap-2">
          <div className="flex-1 px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/[0.03] text-slate-500">
            <span className="font-semibold text-purple-600 dark:text-purple-400">mission</span>
            <span className="text-slate-400 ml-2">— Autonomous mission agent (no human interaction)</span>
          </div>
          <button
            onClick={onViewAgent}
            className="flex items-center gap-1 px-2.5 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors shrink-0"
            title="View mission.md"
          >
            <Eye size={13} /> View
          </button>
        </div>
      </div>

      {/* Model override (optional) */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Model <span className="text-slate-400">(optional — defaults to agent model)</span>
        </label>
        <select
          value={model}
          onChange={(e) => setModel(e.target.value)}
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        >
          <option value="">Default (inherit from agent)</option>
          {models.map(m => (
            <option key={m.id} value={m.id}>
              {m.id} — {m.provider}/{m.model}
            </option>
          ))}
        </select>
      </div>

      {/* Project (optional) */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Project <span className="text-slate-400">(optional — scope mission to a project)</span>
        </label>
        <select
          value={selectedProject}
          onChange={(e) => setSelectedProject(e.target.value)}
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        >
          <option value="">No project (global)</option>
          {projects.map(p => (
            <option key={p.path} value={p.path}>
              {p.name || p.path.split('/').pop()}
            </option>
          ))}
        </select>
      </div>

      {/* Prompt */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Prompt
        </label>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="The instruction to send to the agent on each trigger..."
          rows={6}
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
      </div>

      {/* Actions */}
      <div className="flex items-center gap-3 pt-2">
        <button
          onClick={handleSave}
          disabled={saving || !prompt.trim()}
          className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          {saving ? 'Saving...' : editing ? 'Update Mission' : 'Create Mission'}
        </button>
        <button
          onClick={onCancel}
          className="px-4 py-2 text-sm font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
};

// --- Mission Run History ---

const RunsView: React.FC<{
  mission: CronMission;
  onBack: () => void;
  onOpenSession?: (sessionId: string) => void;
}> = ({ mission, onBack, onOpenSession }) => {
  const [runs, setRuns] = useState<MissionRunEntry[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchMissionRuns(mission.id)
      .then(setRuns)
      .finally(() => setLoading(false));
  }, [mission.id]);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"
        >
          <ArrowLeft size={14} />
        </button>
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          Run History: <span className="font-mono text-blue-600 dark:text-blue-400">{mission.schedule}</span>
        </h2>
      </div>

      <div className="text-xs text-slate-500 mb-2">
        {mission.prompt.slice(0, 100)}{mission.prompt.length > 100 ? '...' : ''}
      </div>

      {loading ? (
        <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
      ) : runs.length === 0 ? (
        <div className="text-center py-16">
          <Clock size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
          <p className="text-sm text-slate-500">No runs yet</p>
        </div>
      ) : (
        <div className="space-y-1.5">
          {[...runs].reverse().map((run, i) => (
            <div
              key={`${run.run_id}-${i}`}
              className={cn(
                'flex items-center gap-3 px-3 py-2 rounded-lg border',
                run.skipped
                  ? 'bg-amber-50/50 dark:bg-amber-500/5 border-amber-200 dark:border-amber-500/10'
                  : run.status === 'completed'
                    ? 'bg-white dark:bg-white/[0.02] border-green-200 dark:border-green-500/10'
                    : 'bg-white dark:bg-white/[0.02] border-slate-200 dark:border-white/10',
              )}
            >
              <span className="text-[10px] text-slate-400 font-mono shrink-0 w-36">
                {formatTimestamp(run.triggered_at)}
              </span>
              <span className={cn(
                'text-[10px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wide',
                run.skipped
                  ? 'bg-amber-500/15 text-amber-600'
                  : run.status === 'completed'
                    ? 'bg-green-500/15 text-green-600'
                    : run.status === 'failed'
                      ? 'bg-red-500/15 text-red-600'
                      : 'bg-slate-500/15 text-slate-500',
              )}>
                {run.skipped ? 'skipped' : run.status}
              </span>
              {run.session_id && !run.skipped && onOpenSession && (
                <button
                  onClick={() => onOpenSession(run.session_id!)}
                  className="flex items-center gap-1 text-[10px] text-blue-500 hover:text-blue-600 font-medium"
                  title="Open session log"
                >
                  <ExternalLink size={10} /> Session
                </button>
              )}
              {run.run_id && !run.skipped && !run.session_id && (
                <span className="text-[10px] text-slate-400 font-mono truncate">
                  {run.run_id}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// --- Agent Viewer (readonly) ---

const AgentViewer: React.FC<{
  onBack: () => void;
  projectRoot: string;
}> = ({ onBack, projectRoot }) => {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    // Try project-local first, then global ~/.linggen/agents/mission.md
    const tryFetch = async () => {
      const localUrl = new URL('/api/agent-file', window.location.origin);
      localUrl.searchParams.append('project_root', projectRoot);
      localUrl.searchParams.append('path', 'agents/mission.md');
      const localResp = await fetch(localUrl.toString());
      if (localResp.ok) {
        const data = await localResp.json();
        if (data.content) return data.content;
      }
      // Fallback to global
      const globalUrl = new URL('/api/agent-file', window.location.origin);
      globalUrl.searchParams.append('project_root', projectRoot);
      globalUrl.searchParams.append('path', '~/.linggen/agents/mission.md');
      const globalResp = await fetch(globalUrl.toString());
      if (globalResp.ok) {
        const data = await globalResp.json();
        if (data.content) return data.content;
      }
      return null;
    };
    tryFetch()
      .then(setContent)
      .catch(() => setContent(null))
      .finally(() => setLoading(false));
  }, [projectRoot]);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"
        >
          <ArrowLeft size={14} />
        </button>
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          Mission Agent <span className="text-[10px] text-slate-400 font-normal ml-1">agents/mission.md</span>
        </h2>
      </div>

      {loading ? (
        <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
      ) : content ? (
        <pre className="text-xs font-mono whitespace-pre-wrap bg-slate-50 dark:bg-white/[0.03] border border-slate-200 dark:border-white/10 rounded-lg p-4 overflow-x-auto text-slate-700 dark:text-slate-300">
          {content}
        </pre>
      ) : (
        <div className="text-center py-16 text-sm text-slate-400">
          Could not load mission.md
        </div>
      )}
    </div>
  );
};

// --- Main Page ---

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  embedded?: boolean;
  onOpenSession?: (sessionId: string) => void;
}> = ({ onBack, projectRoot, agents, embedded, onOpenSession }) => {
  const [tab, setTab] = useState<MissionTab>('list');
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [loading, setLoading] = useState(true);
  const [editingMission, setEditingMission] = useState<CronMission | null>(null);
  const [viewingRunsMission, setViewingRunsMission] = useState<CronMission | null>(null);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);

  // Fetch projects list
  useEffect(() => {
    fetch('/api/projects')
      .then(r => r.ok ? r.json() : [])
      .then((data: ProjectInfo[]) => setProjects(data))
      .catch(() => {});
  }, []);

  const loadMissions = useCallback(async () => {
    const data = await fetchMissions();
    setMissions(data);
    setLoading(false);
  }, []);

  useEffect(() => {
    setLoading(true);
    loadMissions();
  }, [loadMissions]);

  const handleToggle = async (id: string, enabled: boolean) => {
    try {
      await updateMission(id, { enabled });
      await loadMissions();
    } catch (e) {
      console.error('Failed to toggle mission:', e);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteMission(id);
      await loadMissions();
    } catch (e) {
      console.error('Failed to delete mission:', e);
    }
  };

  const handleEdit = (m: CronMission) => {
    setEditingMission(m);
    setTab('edit');
  };

  const handleViewRuns = (m: CronMission) => {
    setViewingRunsMission(m);
    setTab('runs');
  };

  const handleTrigger = async (m: CronMission) => {
    try {
      await triggerMission(m.id, m.project || undefined);
    } catch (e: any) {
      console.error('Failed to trigger mission:', e);
    }
  };

  const handleSave = async (_mission: CronMission) => {
    setEditingMission(null);
    setTab('list');
    await loadMissions();
  };

  const handleCancel = () => {
    setEditingMission(null);
    setTab('list');
  };

  const enabledCount = missions.filter(m => m.enabled).length;

  const tabBar = (
    <div className={cn(
      'flex items-center gap-1 px-6 py-2',
      !embedded && 'border-b border-slate-200 dark:border-white/5 bg-white/50 dark:bg-white/[0.02]',
    )}>
      <button
        onClick={() => { setTab('list'); setEditingMission(null); setViewingRunsMission(null); }}
        className={cn(
          'px-3 py-1.5 rounded-md text-xs font-semibold transition-colors',
          tab === 'list'
            ? 'bg-blue-600 text-white'
            : 'text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-white/5',
        )}
      >
        Missions
      </button>
      {tab === 'create' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          New
        </span>
      )}
      {tab === 'edit' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          Edit
        </span>
      )}
      {tab === 'runs' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          Runs
        </span>
      )}
      {tab === 'agent' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          Agent
        </span>
      )}
    </div>
  );

  const content = (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-4xl mx-auto">
        {loading ? (
          <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
        ) : tab === 'list' ? (
          <MissionList
            missions={missions}
            projects={projects}
            onToggle={handleToggle}
            onEdit={handleEdit}
            onDelete={handleDelete}
            onViewRuns={handleViewRuns}
            onTrigger={handleTrigger}
            onCreate={() => { setEditingMission(null); setTab('create'); }}
          />
        ) : tab === 'create' || tab === 'edit' ? (
          <MissionEditor
            editing={editingMission}
            agents={agents}
            projects={projects}
            onSave={handleSave}
            onCancel={handleCancel}
            onViewAgent={() => setTab('agent')}
          />
        ) : tab === 'runs' && viewingRunsMission ? (
          <RunsView
            mission={viewingRunsMission}
            onBack={() => { setViewingRunsMission(null); setTab('list'); }}
            onOpenSession={onOpenSession}
          />
        ) : tab === 'agent' ? (
          <AgentViewer
            onBack={() => setTab(editingMission ? 'edit' : 'list')}
            projectRoot={projectRoot}
          />
        ) : null}
      </div>
    </div>
  );

  if (embedded) {
    return (
      <div className="flex flex-col h-full">
        {tabBar}
        {content}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={enabledCount > 0 ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Missions</h1>
        </div>
        {enabledCount > 0 && (
          <span className="text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            {enabledCount} active
          </span>
        )}
      </header>

      {tabBar}
      {content}
    </div>
  );
};
