import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import {
  ChevronDown,
  ChevronRight,
  Clock,
  Loader2,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Target,
  X,
} from 'lucide-react';
import { cn } from '../lib/cn';
import type { CronMission, MissionRunEntry, ProjectInfo } from '../types';

// ---- Helpers ----------------------------------------------------------------

const describeCron = (schedule: string): string => {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;
  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${min.slice(2)} min`;
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') return `Every ${hour.slice(2)}h`;
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

const formatShortTime = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  if (isToday) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
};

const statusDot = (run: MissionRunEntry) => {
  if (run.skipped) return 'bg-amber-500';
  if (run.status === 'completed') return 'bg-green-500';
  if (run.status === 'failed') return 'bg-red-500';
  if (run.status === 'running') return 'bg-blue-500 animate-pulse';
  return 'bg-slate-400';
};

// ---- API helpers ------------------------------------------------------------

async function fetchMissions(): Promise<CronMission[]> {
  try {
    const resp = await fetch('/api/missions');
    if (!resp.ok) return [];
    const data = await resp.json();
    return Array.isArray(data.missions) ? data.missions : [];
  } catch {
    return [];
  }
}

async function apiDeleteMission(id: string): Promise<void> {
  await fetch(`/api/missions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

const RUNS_PAGE_SIZE = 20;

async function fetchMissionRuns(id: string, limit?: number, offset?: number): Promise<MissionRunEntry[]> {
  try {
    const params = new URLSearchParams();
    if (limit != null) params.set('limit', String(limit));
    if (offset != null) params.set('offset', String(offset));
    const qs = params.toString();
    const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/runs${qs ? `?${qs}` : ''}`);
    if (!resp.ok) return [];
    const data = await resp.json();
    return Array.isArray(data.runs) ? data.runs : [];
  } catch {
    return [];
  }
}

async function triggerMission(id: string, project?: string): Promise<{ ok: boolean; busy?: boolean; session_id?: string }> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/trigger`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_root: project || null }),
  });
  if (resp.status === 409) return { ok: false, busy: true };
  if (!resp.ok) return { ok: false };
  const data = await resp.json();
  return { ok: true, session_id: data.session_id };
}

async function deleteMissionSession(missionId: string, sessionId: string): Promise<void> {
  await fetch(`/api/missions/${encodeURIComponent(missionId)}/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' });
}

// ---- Component --------------------------------------------------------------

export interface MissionSessionNavProps {
  activeSessionId: string | null;
  setActiveSessionId: (id: string | null, missionId?: string) => void;
  projects: ProjectInfo[];
  /** Navigate to full-page mission editor (create mode). */
  onCreateMission?: () => void;
  /** Navigate to full-page mission editor (edit mode). */
  onEditMission?: (mission: CronMission) => void;
  /** Incrementing this value triggers an immediate mission list refresh. */
  refreshKey?: number;
}

export const MissionSessionNav: React.FC<MissionSessionNavProps> = ({
  activeSessionId,
  setActiveSessionId,
  projects,
  onCreateMission,
  onEditMission,
  refreshKey,
}) => {
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [expandedMissions, setExpandedMissions] = useState<Set<string>>(new Set());
  const [runsByMission, setRunsByMission] = useState<Record<string, MissionRunEntry[]>>({});
  const [loadingRuns, setLoadingRuns] = useState<Set<string>>(new Set());
  const [triggeringId, setTriggeringId] = useState<string | null>(null);
  const [menuMissionId, setMenuMissionId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const menuRef = useRef<HTMLButtonElement>(null);

  // Full reload: missions + runs for all expanded missions
  const reloadAll = useCallback(async () => {
    setRefreshing(true);
    try {
      const ms = await fetchMissions();
      setMissions(ms);
      // Reload runs for currently expanded missions
      const expanded = Array.from(expandedMissions);
      if (expanded.length > 0) {
        const results = await Promise.all(expanded.map(id => fetchMissionRuns(id, RUNS_PAGE_SIZE).then(runs => ({ id, runs }))));
        setRunsByMission(prev => {
          const next = { ...prev };
          for (const { id, runs } of results) next[id] = runs;
          return next;
        });
      }
    } finally {
      setRefreshing(false);
    }
  }, [expandedMissions]);

  // Fetch missions on mount and when refreshKey changes
  useEffect(() => {
    fetchMissions().then(setMissions);
  }, [refreshKey]);

  // Refresh missions periodically (every 30s)
  useEffect(() => {
    const interval = setInterval(() => {
      fetchMissions().then(setMissions);
    }, 30000);
    return () => clearInterval(interval);
  }, []);

  // Close menu on outside click
  useEffect(() => {
    if (!menuMissionId) return;
    const handler = () => setMenuMissionId(null);
    const id = requestAnimationFrame(() => {
      window.addEventListener('click', handler);
    });
    return () => {
      cancelAnimationFrame(id);
      window.removeEventListener('click', handler);
    };
  }, [menuMissionId]);

  const toggleMission = useCallback((missionId: string) => {
    setExpandedMissions(prev => {
      const next = new Set(prev);
      if (next.has(missionId)) {
        next.delete(missionId);
      } else {
        next.add(missionId);
        if (!runsByMission[missionId]) {
          setLoadingRuns(prev => new Set(prev).add(missionId));
          fetchMissionRuns(missionId, RUNS_PAGE_SIZE).then(runs => {
            setRunsByMission(prev => ({ ...prev, [missionId]: runs }));
            setLoadingRuns(prev => {
              const next = new Set(prev);
              next.delete(missionId);
              return next;
            });
          });
        }
      }
      return next;
    });
  }, [runsByMission]);

  const loadMoreRuns = useCallback(async (missionId: string) => {
    const existing = runsByMission[missionId] || [];
    setLoadingRuns(prev => new Set(prev).add(missionId));
    const more = await fetchMissionRuns(missionId, RUNS_PAGE_SIZE, existing.length);
    setRunsByMission(prev => ({
      ...prev,
      [missionId]: [...(prev[missionId] || []), ...more],
    }));
    setLoadingRuns(prev => {
      const next = new Set(prev);
      next.delete(missionId);
      return next;
    });
  }, [runsByMission]);

  const [triggerError, setTriggerError] = useState<string | null>(null);

  const handleTrigger = useCallback(async (mission: CronMission) => {
    setMenuMissionId(null);
    setTriggerError(null);
    setTriggeringId(mission.id);
    const prevRunCount = (runsByMission[mission.id] || []).length;
    try {
      const result = await triggerMission(mission.id, mission.project || undefined);
      if (!result.ok) {
        setTriggeringId(null);
        setTriggerError(result.busy ? 'Mission agent is busy — try again later.' : 'Failed to trigger mission.');
        setTimeout(() => setTriggerError(null), 4000);
        return;
      }
      setExpandedMissions(prev => new Set(prev).add(mission.id));
      // Auto-select the new session and inject a temporary "running" entry
      // so the sidebar shows it immediately (the real run entry is only written
      // after the agent loop completes).
      if (result.session_id) {
        setActiveSessionId(result.session_id, mission.id);
        const tempRun: MissionRunEntry = {
          run_id: `temp-${Date.now()}`,
          session_id: result.session_id,
          triggered_at: Math.floor(Date.now() / 1000),
          status: 'running',
          skipped: false,
        };
        setRunsByMission(prev => ({
          ...prev,
          [mission.id]: [tempRun, ...(prev[mission.id] || [])],
        }));
      }
      // Poll until the real run entry appears (replaces the temp one).
      // Keep the temp entry merged in until the server returns it.
      const tempSessionId = result.session_id;
      const poll = async (retries: number, delay: number) => {
        for (let i = 0; i < retries; i++) {
          await new Promise(r => setTimeout(r, delay));
          const runs = await fetchMissionRuns(mission.id, RUNS_PAGE_SIZE);
          // Check if the real run with our session_id appeared
          const hasReal = tempSessionId && runs.some(r => r.session_id === tempSessionId);
          if (hasReal) {
            setRunsByMission(prev => ({ ...prev, [mission.id]: runs }));
            setTriggeringId(null);
            return;
          }
          // Real run not yet recorded — keep the temp entry at the top
          setRunsByMission(prev => {
            const temp = (prev[mission.id] || []).find(r => r.run_id.startsWith('temp-'));
            return { ...prev, [mission.id]: temp ? [temp, ...runs] : runs };
          });
        }
        // Timeout — remove temp entry
        setRunsByMission(prev => ({
          ...prev,
          [mission.id]: (prev[mission.id] || []).filter(r => !r.run_id.startsWith('temp-')),
        }));
        setTriggeringId(null);
      };
      poll(60, 3000); // poll every 3s for up to 3 minutes
    } catch {
      setTriggeringId(null);
      setTriggerError('Failed to trigger mission.');
      setTimeout(() => setTriggerError(null), 4000);
    }
  }, [runsByMission]);

  const handleDelete = useCallback(async (missionId: string) => {
    setMenuMissionId(null);
    await apiDeleteMission(missionId);
    setMissions(prev => prev.filter(m => m.id !== missionId));
  }, []);

  const handleRunClick = useCallback((missionId: string, run: MissionRunEntry) => {
    if (!run.session_id) return;
    setActiveSessionId(run.session_id, missionId);
  }, [setActiveSessionId]);

  const handleDeleteSession = useCallback(async (missionId: string, run: MissionRunEntry) => {
    if (!run.session_id) return;
    if (run.session_id === activeSessionId) {
      setActiveSessionId(null);
    }
    await deleteMissionSession(missionId, run.session_id);
    setRunsByMission(prev => ({
      ...prev,
      [missionId]: (prev[missionId] || []).filter(r => r.session_id !== run.session_id),
    }));
  }, [activeSessionId, setActiveSessionId]);

  const startEditing = useCallback((mission: CronMission) => {
    setMenuMissionId(null);
    if (onEditMission) onEditMission(mission);
  }, [onEditMission]);

  const displayMissions = useMemo(() => {
    return missions.sort((a, b) => b.created_at - a.created_at);
  }, [missions]);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="p-3 border-b border-slate-200 dark:border-white/5 flex items-center gap-2">
        <div className="flex-1 flex items-center gap-1.5">
          <Target size={13} className="text-slate-400" />
          <span className="text-[11px] font-bold text-slate-600 dark:text-slate-300 uppercase tracking-wider">
            Missions
          </span>
          <span className="text-[10px] text-slate-400 ml-1">
            {missions.length}
          </span>
        </div>
        <button
          onClick={reloadAll}
          disabled={refreshing}
          className="p-1.5 rounded-lg transition-colors hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 hover:text-blue-500 disabled:opacity-50"
          title="Refresh missions & sessions"
        >
          <RefreshCw size={13} className={refreshing ? 'animate-spin' : ''} />
        </button>
        <button
          onClick={() => onCreateMission?.()}
          className="p-1.5 rounded-lg transition-colors hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 hover:text-blue-500"
          title="New Mission"
        >
          <Plus size={13} />
        </button>
      </div>

      {/* Error banner */}
      {triggerError && (
        <div className="mx-2 mt-2 px-2.5 py-1.5 rounded-lg bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/20 text-[11px] text-amber-700 dark:text-amber-400">
          {triggerError}
        </div>
      )}

      {/* Mission tree (scrollable) */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {displayMissions.length === 0 && (
          <div className="p-4 text-xs text-slate-500 italic text-center">
            No missions yet. Click + to create one.
          </div>
        )}

        {displayMissions.map(mission => {
          const isExpanded = expandedMissions.has(mission.id);
          const runs = (runsByMission[mission.id] || []).slice().sort((a, b) => b.triggered_at - a.triggered_at);
          const isLoading = loadingRuns.has(mission.id);
          const isMenuOpen = menuMissionId === mission.id;
          const projLabel = mission.project
            ? (projects.find(p => p.path === mission.project)?.name || mission.project.split('/').pop())
            : null;

          return (
            <div key={mission.id} className="rounded-lg">
              {/* Mission header */}
              <div className="relative group">
                <button
                  onClick={() => toggleMission(mission.id)}
                  className={cn(
                    'w-full text-left px-2.5 py-2 rounded-lg transition-colors',
                    'hover:bg-slate-50 dark:hover:bg-white/5',
                  )}
                >
                  <div className="flex items-center gap-1.5">
                    {isExpanded ? (
                      <ChevronDown size={13} className="text-slate-400 shrink-0" />
                    ) : (
                      <ChevronRight size={13} className="text-slate-400 shrink-0" />
                    )}
                    {triggeringId === mission.id ? (
                      <Loader2 size={13} className="animate-spin text-blue-500 shrink-0" />
                    ) : runs.some(r => r.run_id.startsWith('temp-')) ? (
                      <span className="inline-block w-2 h-2 rounded-full shrink-0 bg-blue-500 animate-pulse" />
                    ) : (
                      <span className={cn(
                        'inline-block w-1.5 h-1.5 rounded-full shrink-0',
                        mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
                      )} />
                    )}
                    <span className="text-[11px] font-bold text-slate-800 dark:text-slate-200 truncate">
                      {mission.name || mission.prompt.slice(0, 40)}
                    </span>
                  </div>
                  <div className="ml-5 flex items-center gap-2 mt-0.5">
                    <span className="text-[10px] font-mono text-blue-600 dark:text-blue-400 flex items-center gap-1">
                      <Clock size={9} />
                      {describeCron(mission.schedule)}
                    </span>
                    {projLabel && (
                      <span className="text-[10px] text-slate-400 truncate">
                        {projLabel}
                      </span>
                    )}
                  </div>
                </button>

                {/* "..." context menu trigger */}
                <button
                  ref={isMenuOpen ? menuRef : undefined}
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuMissionId(isMenuOpen ? null : mission.id);
                  }}
                  className="absolute right-2 top-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 transition-all"
                >
                  <MoreHorizontal size={13} />
                </button>

                {/* Dropdown menu */}
                {isMenuOpen && (
                  <div
                    className="absolute right-2 top-8 z-50 bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl py-1 min-w-[120px]"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      onClick={() => startEditing(mission)}
                      className="w-full text-left px-3 py-1.5 text-[11px] hover:bg-slate-50 dark:hover:bg-white/5"
                    >
                      Edit
                    </button>
                    <button
                      onClick={() => handleTrigger(mission)}
                      disabled={triggeringId === mission.id}
                      className="w-full text-left px-3 py-1.5 text-[11px] hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50 flex items-center gap-1.5"
                    >
                      {triggeringId === mission.id && <Loader2 size={11} className="animate-spin" />}
                      {triggeringId === mission.id ? 'Running...' : 'Run Now'}
                    </button>
                    <button
                      onClick={() => handleDelete(mission.id)}
                      className="w-full text-left px-3 py-1.5 text-[11px] text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10"
                    >
                      Delete
                    </button>
                  </div>
                )}
              </div>

              {/* Runs list */}
              {isExpanded && (
                <div className="ml-3 mt-0.5 space-y-0.5">
                  {isLoading && (
                    <div className="px-2.5 py-2 text-[10px] text-slate-400 italic">Loading runs...</div>
                  )}
                  {!isLoading && runs.length === 0 && (
                    <div className="px-2.5 py-2 text-[10px] text-slate-400 italic">No runs yet</div>
                  )}
                  {runs.map(run => {
                    const isActive = run.session_id === activeSessionId;
                    return (
                      <div key={run.run_id || `run-${run.triggered_at}`} className="relative group/run">
                        <button
                          onClick={() => handleRunClick(mission.id, run)}
                          disabled={!run.session_id}
                          className={cn(
                            'w-full text-left px-2.5 py-2 rounded-lg transition-colors text-[11px]',
                            isActive
                              ? 'bg-blue-100/80 dark:bg-blue-500/15 border-l-2 border-blue-500'
                              : 'hover:bg-slate-50 dark:hover:bg-white/5',
                            !run.session_id && 'opacity-50 cursor-not-allowed',
                          )}
                        >
                          <div className="flex items-center gap-1.5">
                            <span className={cn('inline-block w-1.5 h-1.5 rounded-full shrink-0', statusDot(run))} />
                            <span className="font-medium text-slate-800 dark:text-slate-200 truncate">
                              {run.skipped ? 'Skipped' : run.status}
                            </span>
                            <span className="text-[10px] text-slate-400 ml-auto shrink-0 pr-5">
                              {formatShortTime(run.triggered_at)}
                            </span>
                          </div>
                        </button>
                        {run.session_id && (
                          <button
                            onClick={(e) => { e.stopPropagation(); handleDeleteSession(mission.id, run); }}
                            className="absolute right-2 top-1/2 -translate-y-1/2 p-0.5 rounded opacity-0 group-hover/run:opacity-100 hover:bg-red-100 dark:hover:bg-red-500/20 text-slate-400 hover:text-red-500 transition-all"
                            title="Delete session"
                          >
                            <X size={11} />
                          </button>
                        )}
                      </div>
                    );
                  })}
                  {/* Load more button — shown when we got a full page (more may exist) */}
                  {runs.length > 0 && runs.length % RUNS_PAGE_SIZE === 0 && (
                    <button
                      onClick={() => loadMoreRuns(mission.id)}
                      disabled={isLoading}
                      className="w-full px-2.5 py-1.5 text-[10px] text-blue-500 hover:text-blue-600 hover:bg-slate-50 dark:hover:bg-white/5 rounded-lg transition-colors disabled:opacity-50"
                    >
                      {isLoading ? 'Loading...' : 'Load more'}
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};
