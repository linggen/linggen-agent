import React, { useState, useEffect, useCallback, useRef } from 'react';
import { ArrowLeft, Target, Plus, Trash2, Check, X, Eye } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, CronMission } from '../types';
import { useSessionStore } from '../stores/sessionStore';

// ---- Helpers ----------------------------------------------------------------

const describeCron = (schedule: string): string => {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;
  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${min.slice(2)} min`;
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') return `Every ${hour.slice(2)}h at :${min.padStart(2, '0')}`;
  if (hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Hourly at :${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow === '*') return `Daily ${hour}:${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow !== '*') {
    const dayNames: Record<string, string> = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun' };
    if (dow.includes('-')) {
      const [start, end] = dow.split('-');
      return `${dayNames[start] || start}-${dayNames[end] || end} ${hour}:${min.padStart(2, '0')}`;
    }
    const days = dow.split(',').map(d => dayNames[d] || d).join(', ');
    return `${days} ${hour}:${min.padStart(2, '0')}`;
  }
  return schedule;
};

/** Display label for a working folder path — show the last segment. */
const folderLabel = (path: string | null | undefined): string | null => {
  if (!path) return null;
  return path.split('/').pop() || path;
};

// ---- API helpers ------------------------------------------------------------

async function fetchMissions(): Promise<CronMission[]> {
  const resp = await fetch('/api/missions');
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.missions) ? data.missions : [];
}

interface CreateMissionArgs {
  name?: string;
  description?: string;
  schedule: string;
  prompt?: string;
  model?: string;
  cwd?: string;
  entry?: string;
  policy?: string;
  permission_mode?: string;
  permission_paths?: string[];
  permission_warning?: string;
  allow_skills?: string[];
  requires?: string[];
  allowed_tools?: string[];
}

async function createMission(args: CreateMissionArgs): Promise<CronMission | null> {
  const resp = await fetch('/api/missions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      name: args.name || null,
      description: args.description || '',
      schedule: args.schedule,
      prompt: args.prompt || null,
      model: args.model || null,
      cwd: args.cwd || null,
      entry: args.entry || null,
      policy: args.policy || 'strict',
      permission_mode: args.permission_mode || 'admin',
      permission_paths: args.permission_paths || [],
      permission_warning: args.permission_warning || null,
      allow_skills: args.allow_skills || [],
      requires: args.requires || [],
      allowed_tools: args.allowed_tools || [],
    }),
  });
  if (!resp.ok) { throw new Error(await resp.text()); }
  return resp.json();
}

async function updateMission(id: string, updates: Record<string, any>): Promise<CronMission | null> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(updates),
  });
  if (!resp.ok) { throw new Error(await resp.text()); }
  return resp.json();
}

async function deleteMission(id: string): Promise<void> {
  await fetch(`/api/missions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}



// ---- Left Sidebar: Mission Nav ---------------------------------------------

/** Selected item in the sidebar: inline editor (existing or new), agent viewer, or nothing.
 *  Run sessions are viewed via the main-page session list, not here. */
type SidebarSelection =
  | { type: 'editor'; missionId: string | null }
  | { type: 'agent-viewer' }
  | null;

const MissionNav: React.FC<{
  missions: CronMission[];
  selection: SidebarSelection;
  onSelectMission: (m: CronMission) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  onDelete: (id: string) => void;
  onCreate: () => void;
}> = ({ missions, selection, onSelectMission, onToggleEnabled: _onToggleEnabled, onDelete, onCreate }) => {
  const enabledCount = missions.filter(m => m.enabled).length;
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="p-3 border-b border-slate-200 dark:border-white/5 flex items-center gap-2">
        <button
          onClick={onCreate}
          className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-[12px] font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors"
        >
          <Plus size={13} /> New Mission
        </button>
        {enabledCount > 0 && (
          <span className="text-[10px] font-bold px-1.5 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400 shrink-0">
            {enabledCount} active
          </span>
        )}
      </div>

      {/* Mission list (scrollable) */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {missions.length === 0 && (
          <div className="p-4 text-xs text-slate-500 italic text-center">
            No missions yet. Create one to start.
          </div>
        )}

        {missions.map(mission => {
          const projLabel = folderLabel(mission.cwd || mission.project);

          return (
            <div key={mission.id} className="rounded-lg">
              {/* Mission header. Per-mission run history was removed — run
                  sessions now live in the main page's unified session list
                  under the "Mission" tab. */}
              <div className={cn(
                'relative group rounded-lg transition-colors',
                selection?.type === 'editor' && selection.missionId === mission.id
                  ? 'bg-blue-50 dark:bg-blue-500/10'
                  : 'hover:bg-slate-50 dark:hover:bg-white/5',
              )}>
                <div className="flex items-stretch">
                  <button
                    onClick={() => onSelectMission(mission)}
                    className="flex-1 text-left px-2.5 py-2 min-w-0"
                  >
                    <div className="flex items-center gap-1.5">
                      <span className={cn(
                        'w-2 h-2 rounded-full shrink-0',
                        mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
                      )} />
                      <span className="text-[12px] font-bold text-slate-800 dark:text-slate-200 truncate">
                        {mission.name || 'Untitled Mission'}
                      </span>
                    </div>
                    <div className="text-[11px] text-slate-400 truncate mt-0.5">
                      {describeCron(mission.schedule)}
                      {projLabel && <> &middot; {projLabel}</>}
                    </div>
                  </button>
                </div>

                {/* Action buttons on hover. Run is intentionally absent —
                    trigger missions from the main-page session list instead. */}
                <div className="absolute right-1.5 top-1.5 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-all">
                  {confirmDeleteId === mission.id ? (
                    <>
                      <button
                        onClick={(e) => { e.stopPropagation(); onDelete(mission.id); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-red-500"
                        title="Confirm delete"
                      >
                        <Check size={11} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400"
                        title="Cancel"
                      >
                        <X size={11} />
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(mission.id); }}
                      className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500"
                      title="Delete"
                    >
                      <Trash2 size={11} />
                    </button>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ---- Mission Editor ---------------------------------------------------------

const CRON_PRESETS = [
  { label: 'Every 30 min', value: '*/30 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 2 hours', value: '0 */2 * * *' },
  { label: 'Daily at 9am', value: '0 9 * * *' },
  { label: 'Weekdays 9am', value: '0 9 * * 1-5' },
  { label: 'Weekly Sunday', value: '0 0 * * 0' },
];

export const PERMISSION_MODES = [
  { value: 'read', label: 'Read-only', desc: 'Analyze and report only — no Write, Edit, or Bash.', color: 'green' },
  { value: 'edit', label: 'Edit', desc: 'Write + edit files within the working directory and declared paths.', color: 'blue' },
  { value: 'admin', label: 'Admin', desc: 'Full access, no restrictions. Use with caution.', color: 'amber' },
] as const;

export const POLICY_OPTIONS = [
  { value: 'strict',      label: 'Strict',      desc: 'Silently deny anything outside the grant. Safest default for headless runs.',  color: 'green' },
  { value: 'trusted',     label: 'Trusted',     desc: 'Silently allow out-of-scope. Ask-rules (git push, etc.) still deny.',           color: 'blue' },
  { value: 'sandbox',     label: 'Sandbox',     desc: 'Allow everything. For Docker/VM runs where the OS is the guardrail.',          color: 'amber' },
  { value: 'interactive', label: 'Interactive', desc: 'Prompt for out-of-scope. Discouraged for missions — prompts queue unseen.',    color: 'red'   },
] as const;

export const MissionEditor: React.FC<{
  editing: CronMission | null;
  workingFolders: string[];
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
  onViewAgent: () => void;
}> = ({ editing, workingFolders, onSave, onCancel, onViewAgent }) => {
  const [name, setName] = useState(editing?.name || '');
  const [description, setDescription] = useState(editing?.description || '');
  const [schedule, setSchedule] = useState(editing?.schedule || '*/30 * * * *');
  const [prompt, setPrompt] = useState(editing?.prompt || '');
  const [model, setModel] = useState(editing?.model || '');
  const [selectedCwd, setSelectedCwd] = useState(editing?.cwd || editing?.project || '');
  const [entry, setEntry] = useState(editing?.entry || '');

  const [policy, setPolicy] = useState<'strict' | 'trusted' | 'sandbox' | 'interactive'>(
    ((editing?.policy as 'strict' | 'trusted' | 'sandbox' | 'interactive') || 'strict'),
  );
  const [permissionMode, setPermissionMode] = useState<'read' | 'edit' | 'admin'>(
    (editing?.permission?.mode as 'read' | 'edit' | 'admin') || 'admin',
  );
  const [permissionPathsText, setPermissionPathsText] = useState(
    (editing?.permission?.paths || []).join('\n'),
  );
  const [permissionWarning, setPermissionWarning] = useState(editing?.permission?.warning || '');

  const [allowSkillsText, setAllowSkillsText] = useState((editing?.allow_skills || []).join(', '));
  const [requiresText, setRequiresText] = useState((editing?.requires || []).join(', '));
  const [allowedToolsText, setAllowedToolsText] = useState((editing?.allowed_tools || []).join(', '));

  const [models, setModels] = useState<{ id: string; model: string; provider: string }[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetch('/api/config').then(r => r.ok ? r.json() : null).then(data => { if (data?.models) setModels(data.models); }).catch(() => {});
  }, []);

  const splitList = (s: string): string[] =>
    s.split(/[\s,]+/).map(x => x.trim()).filter(Boolean);

  const splitLines = (s: string): string[] =>
    s.split('\n').map(x => x.trim()).filter(Boolean);

  const handleSave = async () => {
    if (!schedule.trim()) { setError('Schedule is required'); return; }
    if (!prompt.trim() && !entry.trim()) {
      setError('Mission needs either a prompt body or an entry script');
      return;
    }
    setSaving(true); setError(null);
    try {
      const permPaths = splitLines(permissionPathsText);
      const payload = {
        name: name || undefined,
        description: description || '',
        schedule,
        prompt: prompt || undefined,
        model: model || undefined,
        cwd: selectedCwd || undefined,
        entry: entry || undefined,
        policy,
        permission_mode: permissionMode,
        permission_paths: permPaths,
        permission_warning: permissionWarning || undefined,
        allow_skills: splitList(allowSkillsText),
        requires: splitList(requiresText),
        allowed_tools: splitList(allowedToolsText),
      };
      const result = editing
        ? await updateMission(editing.id, payload)
        : await createMission(payload);
      if (result) onSave(result);
    } catch (e: any) { setError(e.message || 'Failed to save mission'); }
    setSaving(false);
  };

  const scriptOnly = !prompt.trim() && entry.trim().length > 0;

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          {editing ? 'Edit Mission' : 'New Mission'}
        </h2>

        {error && <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-3 text-xs text-red-600 dark:text-red-400">{error}</div>}

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Name</label>
          <input type="text" value={name} onChange={e => setName(e.target.value)} placeholder="e.g. dream"
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Description</label>
          <textarea value={description} onChange={e => setDescription(e.target.value)} placeholder="One-line summary — shown in the mission list" rows={2}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Cron Schedule</label>
          <input type="text" value={schedule} onChange={e => setSchedule(e.target.value)} placeholder="*/30 * * * *"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
          <div className="flex flex-wrap gap-1.5 mt-2">
            {CRON_PRESETS.map(p => (
              <button key={p.value} onClick={() => setSchedule(p.value)} className={cn(
                'text-[11px] px-2 py-0.5 rounded-full border transition-colors',
                schedule === p.value ? 'border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400' : 'border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5',
              )}>{p.label}</button>
            ))}
          </div>
          <div className="text-[11px] text-slate-400 mt-1.5">{describeCron(schedule)}</div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Agent</label>
          <div className="flex items-center gap-2">
            <div className="flex-1 px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/[0.03] text-slate-500">
              <span className="font-semibold text-purple-600 dark:text-purple-400">ling</span>
              <span className="text-slate-400 ml-2">— Autonomous (no AskUser, no UI)</span>
            </div>
            <button onClick={onViewAgent} className="flex items-center gap-1 px-2.5 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors shrink-0" title="View mission.md">
              <Eye size={13} /> View
            </button>
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Entry script <span className="text-slate-400 font-normal">(optional — runs before the agent)</span>
          </label>
          <input type="text" value={entry} onChange={e => setEntry(e.target.value)}
            placeholder="scripts/collect.sh  or  inline bash -c '...'"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
          <div className="text-[11px] text-slate-400 mt-1.5">
            Relative paths resolve against the mission directory. Entry receives <code>MISSION_ID</code>, <code>MISSION_DIR</code>, <code>MISSION_CWD</code>, <code>MISSION_OUTPUT_DIR</code>, <code>MISSION_LAST_RUN_AT</code>, <code>MISSION_RUN_ID</code>.
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Autonomy policy
          </label>
          <div className="space-y-2">
            {POLICY_OPTIONS.map(opt => {
              const selected = policy === opt.value;
              const colorMap: Record<string, string> = {
                blue: selected ? 'border-blue-500/40 bg-blue-500/10' : '',
                green: selected ? 'border-green-500/40 bg-green-500/10' : '',
                amber: selected ? 'border-amber-500/40 bg-amber-500/10' : '',
                red: selected ? 'border-red-500/40 bg-red-500/10' : '',
              };
              const dotMap: Record<string, string> = {
                blue: 'bg-blue-500', green: 'bg-green-500',
                amber: 'bg-amber-500', red: 'bg-red-500',
              };
              return (
                <button key={opt.value} type="button" onClick={() => setPolicy(opt.value as any)}
                  className={cn(
                    'w-full flex items-start gap-3 px-3 py-2.5 rounded-lg border text-left transition-colors',
                    selected ? colorMap[opt.color] : 'border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5',
                  )}>
                  <div className={cn('w-3 h-3 rounded-full mt-0.5 shrink-0 border-2',
                    selected ? dotMap[opt.color] + ' border-transparent' : 'border-slate-300 dark:border-white/20')} />
                  <div className="min-w-0">
                    <div className="text-xs font-semibold text-slate-700 dark:text-slate-200">{opt.label}</div>
                    <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">{opt.desc}</div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Permission ceiling</label>
          <div className="space-y-2">
            {PERMISSION_MODES.map(m => {
              const selected = permissionMode === m.value;
              const colorMap: Record<string, string> = {
                green: selected ? 'border-green-500/40 bg-green-500/10' : '',
                blue: selected ? 'border-blue-500/40 bg-blue-500/10' : '',
                amber: selected ? 'border-amber-500/40 bg-amber-500/10' : '',
              };
              const dotMap: Record<string, string> = {
                green: 'bg-green-500', blue: 'bg-blue-500', amber: 'bg-amber-500',
              };
              return (
                <button key={m.value} type="button" onClick={() => setPermissionMode(m.value as any)}
                  className={cn(
                    'w-full flex items-start gap-3 px-3 py-2.5 rounded-lg border text-left transition-colors',
                    selected ? colorMap[m.color] : 'border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5',
                  )}>
                  <div className={cn('w-3 h-3 rounded-full mt-0.5 shrink-0 border-2',
                    selected ? dotMap[m.color] + ' border-transparent' : 'border-slate-300 dark:border-white/20')} />
                  <div className="min-w-0">
                    <div className="text-xs font-semibold text-slate-700 dark:text-slate-200">{m.label}</div>
                    <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">{m.desc}</div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Permission paths <span className="text-slate-400 font-normal">(one per line — extra grants beyond cwd)</span>
          </label>
          <textarea value={permissionPathsText} onChange={e => setPermissionPathsText(e.target.value)}
            placeholder={'~/.linggen/memory\n~/.claude/projects'} rows={3}
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Permission warning <span className="text-slate-400 font-normal">(shown in UI before enabling)</span>
          </label>
          <input type="text" value={permissionWarning} onChange={e => setPermissionWarning(e.target.value)}
            placeholder="e.g. Reads session files and writes to ~/.linggen/memory"
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Allowed tools <span className="text-slate-400 font-normal">(comma-separated; empty = unrestricted)</span>
          </label>
          <input type="text" value={allowedToolsText} onChange={e => setAllowedToolsText(e.target.value)}
            placeholder="Read, Write, Edit, Bash, Glob, Grep, Task, Memory_add, Memory_search"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Allow skills <span className="text-slate-400 font-normal">(via Skill tool — empty removes Skill, * allows any)</span>
          </label>
          <input type="text" value={allowSkillsText} onChange={e => setAllowSkillsText(e.target.value)}
            placeholder="memory, linggen"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Requires <span className="text-slate-400 font-normal">(capabilities — validated at load)</span>
          </label>
          <input type="text" value={requiresText} onChange={e => setRequiresText(e.target.value)}
            placeholder="memory"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Model <span className="text-slate-400">(optional)</span></label>
          <select value={model} onChange={e => setModel(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">Default (inherit from agent)</option>
            {models.map(m => <option key={m.id} value={m.id}>{m.id} — {m.provider}/{m.model}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Working folder <span className="text-slate-400 font-normal">(optional, defaults to HOME)</span>
          </label>
          <select value={selectedCwd} onChange={e => setSelectedCwd(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">HOME (default)</option>
            {workingFolders.map(p => <option key={p} value={p}>{p.split('/').pop() || p}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Prompt body {scriptOnly && <span className="text-amber-600 dark:text-amber-400">(empty — this is a script-only mission)</span>}
          </label>
          <textarea value={prompt} onChange={e => setPrompt(e.target.value)}
            placeholder="Step-by-step instructions for the agent. Leave empty for a script-only mission." rows={8}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div className="flex items-center gap-3 pt-2">
          <button onClick={handleSave} disabled={saving || (!prompt.trim() && !entry.trim())}
            className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            {saving ? 'Saving...' : editing ? 'Update Mission' : 'Create Mission'}
          </button>
          <button onClick={onCancel}
            className="px-4 py-2 text-sm font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors">
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};

// ---- Agent Viewer (readonly) ------------------------------------------------

const AgentViewer: React.FC<{ onBack: () => void; projectRoot: string }> = ({ onBack, projectRoot }) => {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const tryFetch = async () => {
      for (const path of ['agents/mission.md', '~/.linggen/agents/mission.md']) {
        const url = new URL('/api/agent-file', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        url.searchParams.append('path', path);
        const resp = await fetch(url.toString());
        if (resp.ok) { const data = await resp.json(); if (data.content) return data.content; }
      }
      return null;
    };
    tryFetch().then(setContent).catch(() => setContent(null)).finally(() => setLoading(false));
  }, [projectRoot]);

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <div className="flex items-center gap-3">
          <button onClick={onBack} className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"><ArrowLeft size={14} /></button>
          <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
            Mission Agent <span className="text-[11px] text-slate-400 font-normal ml-1">agents/mission.md</span>
          </h2>
        </div>
        {loading ? <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
          : content ? <pre className="text-xs font-mono whitespace-pre-wrap bg-slate-50 dark:bg-white/[0.03] border border-slate-200 dark:border-white/10 rounded-lg p-4 overflow-x-auto text-slate-700 dark:text-slate-300">{content}</pre>
          : <div className="text-center py-16 text-sm text-slate-400">Could not load mission.md</div>
        }
      </div>
    </div>
  );
};

// ---- Right Panel: empty-state placeholder ----------------------------------
//
// The mission page no longer views run sessions inline — mission run sessions
// are in the main page's unified session list under the "Mission" tab. This
// panel just shows a placeholder when nothing is being edited.

const RightPanel: React.FC<{
  selection: SidebarSelection;
}> = ({ selection }) => {
  if (!selection) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
        <Target size={32} className="mb-3 opacity-30" />
        <p className="text-sm">Select a mission to edit, or create a new one.</p>
        <p className="text-[12px] mt-1 text-slate-400">Run sessions appear in the main page's session list under the Mission tab.</p>
      </div>
    );
  }
  return null;
};

// ---- Main Page --------------------------------------------------------------

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  embedded?: boolean;
  onOpenSession?: (sessionId: string) => void;
}> = ({ onBack: rawOnBack, projectRoot, agents: _agents, embedded, onOpenSession: _onOpenSession }) => {
  // Reset mission session state when navigating away
  const onBack = useCallback(() => {
    useSessionStore.setState({ isMissionSession: false, activeMissionId: null });
    rawOnBack();
  }, [rawOnBack]);
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [loading, setLoading] = useState(true);
  const [selection, setSelection] = useState<SidebarSelection>(null);
  const [rightView, setRightView] = useState<'session' | 'editor' | 'agent-viewer'>('session');
  const [editingMission, setEditingMission] = useState<CronMission | null>(null);

  // Derive unique working folders from sessions' cwd field
  const allSessions = useSessionStore((s) => s.allSessions);
  const workingFolders = React.useMemo(() => {
    const folders = new Set<string>();
    for (const s of allSessions) {
      if (s.cwd) folders.add(s.cwd);
    }
    return [...folders].sort();
  }, [allSessions]);

  const loadMissions = useCallback(async () => {
    const data = await fetchMissions();
    setMissions(data);
    setLoading(false);
  }, []);

  useEffect(() => { setLoading(true); loadMissions(); }, [loadMissions]);

  const handleToggle = async (id: string, enabled: boolean) => {
    try { await updateMission(id, { enabled }); await loadMissions(); } catch (e) { console.error('Failed to toggle mission:', e); }
  };

  const handleDelete = async (id: string) => {
    try { await deleteMission(id); await loadMissions(); if (selection && 'missionId' in selection && selection.missionId === id) setSelection(null); } catch (e) { console.error('Failed to delete mission:', e); }
  };

  const handleSelectMission = (m: CronMission) => {
    setEditingMission(m);
    setSelection({ type: 'editor', missionId: m.id });
    setRightView('editor');
  };

  const handleCreate = () => {
    setEditingMission(null);
    setSelection({ type: 'editor', missionId: null });
    setRightView('editor');
  };

  const handleSave = async (mission: CronMission) => {
    await loadMissions();
    setEditingMission(mission);
    setSelection({ type: 'editor', missionId: mission.id });
    setRightView('editor');
  };

  const handleCancel = () => {
    setEditingMission(null);
    setSelection(null);
    setRightView('session');
  };

  const handleViewAgent = () => {
    setSelection({ type: 'agent-viewer' });
    setRightView('agent-viewer');
  };

  const enabledCount = missions.filter(m => m.enabled).length;

  const mainContent = (
    <div className="flex-1 flex overflow-hidden">
      {/* Left sidebar — mission nav */}
      <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
        <MissionNav
          missions={missions}
          selection={selection}
          onSelectMission={handleSelectMission}
          onToggleEnabled={handleToggle}
          onDelete={handleDelete}
          onCreate={handleCreate}
        />
      </div>

      {/* Right panel — session content or full-page editor */}
      <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
        {loading ? (
          <div className="flex-1 flex items-center justify-center text-sm text-slate-400">Loading...</div>
        ) : rightView === 'editor' ? (
          <MissionEditor
            editing={editingMission}
            workingFolders={workingFolders}
            onSave={handleSave}
            onCancel={handleCancel}
            onViewAgent={handleViewAgent}
          />
        ) : rightView === 'agent-viewer' ? (
          <AgentViewer
            onBack={() => { setRightView(editingMission ? 'editor' : 'session'); }}
            projectRoot={projectRoot}
          />
        ) : (
          <RightPanel selection={selection} />
        )}
      </main>
    </div>
  );

  if (embedded) {
    return <div className="flex flex-col h-full">{mainContent}</div>;
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md shrink-0">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={enabledCount > 0 ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Missions</h1>
        </div>
        {enabledCount > 0 && (
          <span className="text-[11px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            {enabledCount} active
          </span>
        )}
      </header>
      {mainContent}
    </div>
  );
};
