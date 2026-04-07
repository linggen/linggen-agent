/**
 * Compact per-session model and permission mode selectors.
 * Extracted from ChatPanel.tsx.
 */
import React from 'react';
import { useAgentStore } from '../../stores/agentStore';
import { useUiStore } from '../../stores/uiStore';
import { useProjectStore } from '../../stores/projectStore';

/** Compact per-session model selector shown in the run bar. */
export const SessionModelSelector: React.FC = () => {
  const models = useAgentStore((s) => s.models);
  const defaultModels = useAgentStore((s) => s.defaultModels);
  const sessionModel = useUiStore((s) => s.sessionModel);
  const setSessionModel = useUiStore((s) => s.setSessionModel);
  const sessionId = useProjectStore((s) => s.activeSessionId);
  const selectedProjectRoot = useProjectStore((s) => s.selectedProjectRoot);

  const defaultLabel = defaultModels.length > 0 ? defaultModels[0] : 'default';

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value || null;
    setSessionModel(value);
    if (sessionId) {
      const ps = useProjectStore.getState();
      const updated = ps.allSessions.map((s) =>
        s.id === sessionId ? { ...s, model_id: value } : s
      );
      const updatedSessions = ps.sessions.map((s) =>
        s.id === sessionId ? { ...s, model_id: value } : s
      );
      useProjectStore.setState({ allSessions: updated, sessions: updatedSessions });
      fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot || '',
          session_id: sessionId,
          model_id: value ?? '',
        }),
      }).catch(() => {});
    }
  };

  return (
    <select
      value={sessionModel ?? ''}
      onChange={handleChange}
      onClick={(e) => e.stopPropagation()}
      className="ml-auto text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none max-w-[14rem] truncate"
      title="Session model override"
    >
      <option value="">Default ({defaultLabel})</option>
      {models.filter((m) => !defaultModels.includes(m.id)).map((m) => (
        <option key={m.id} value={m.id}>{m.id}</option>
      ))}
    </select>
  );
};

/** Compact per-session permission mode selector shown in the run bar. */
export const SessionModeSelector: React.FC = () => {
  const sessionMode = useUiStore((s) => s.sessionMode);
  const setSessionMode = useUiStore((s) => s.setSessionMode);
  const permissionVersion = useUiStore((s) => s.permissionVersion);
  const sessionId = useProjectStore((s) => s.activeSessionId);
  const [zone, setZone] = React.useState<string>('home');
  const skipRefetchUntil = React.useRef(0);

  const modes = [
    { value: 'read', label: 'read', color: 'text-emerald-600 dark:text-emerald-400' },
    { value: 'edit', label: 'edit', color: 'text-blue-600 dark:text-blue-400' },
    { value: 'admin', label: 'admin', color: 'text-amber-600 dark:text-amber-400' },
  ];

  const isSystemZone = zone === 'system';

  React.useEffect(() => {
    if (!sessionId) return;
    if (Date.now() < skipRefetchUntil.current) return;
    const sessionMeta = useProjectStore.getState().allSessions.find((s) => s.id === sessionId);
    const cwd = sessionMeta?.cwd || sessionMeta?.project || '';
    const params = new URLSearchParams({ session_id: sessionId });
    if (cwd) params.set('cwd', cwd);
    fetch(`/api/sessions/permission?${params}`)
      .then((r) => r.ok ? r.json() : null)
      .then((resp) => {
        if (Date.now() < skipRefetchUntil.current) return;
        if (resp?.effective_mode) {
          setSessionMode(resp.effective_mode);
        } else if (resp?.path_modes?.length > 0) {
          setSessionMode(resp.path_modes[0].mode);
        } else {
          setSessionMode('read');
        }
        setZone(resp?.zone || 'home');
      })
      .catch(() => {
        if (Date.now() < skipRefetchUntil.current) return;
        setSessionMode('read'); setZone('home');
      });
  }, [sessionId, permissionVersion, setSessionMode]);

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value;
    setSessionMode(value);
    skipRefetchUntil.current = Date.now() + 2000;
    if (sessionId) {
      const sessionMeta = useProjectStore.getState().allSessions.find((s) => s.id === sessionId);
      const cwd = sessionMeta?.cwd || sessionMeta?.project || '~/';
      fetch('/api/sessions/permission', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ session_id: sessionId, path: cwd, mode: value }),
      }).catch(() => {});
    }
  };

  const current = modes.find((m) => m.value === (sessionMode || 'read'));

  if (isSystemZone) {
    return (
      <span
        className="text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 font-semibold bg-slate-50 dark:bg-black/30 text-slate-400 dark:text-slate-500"
        title="System path — read only, no mode switch"
      >
        read (system)
      </span>
    );
  }

  return (
    <select
      value={sessionMode || 'read'}
      onChange={handleChange}
      onClick={(e) => e.stopPropagation()}
      className={`text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none font-semibold bg-white dark:bg-black/30 ${current?.color || ''}`}
      title="Session permission mode"
    >
      {modes.map((m) => (
        <option key={m.value} value={m.value}>{m.label}</option>
      ))}
    </select>
  );
};
