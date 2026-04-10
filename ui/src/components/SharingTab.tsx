import React, { useCallback, useEffect, useState } from 'react';
import { ExternalLink, Copy, Check, Trash2, RefreshCw, Users } from 'lucide-react';

interface RoomData {
  id: string;
  name: string;
  room_type: string;
  instance_id: string;
  invite_token: string | null;
  max_consumers: number;
  token_budget_daily: number | null;
  tokens_used_today: number;
  online: boolean;
  member_count: number;
}

interface Member {
  user_id: string;
  display_name: string;
  avatar_url: string | null;
  consumer_type: string;
  added_at: string;
}

export const SharingTab: React.FC = () => {
  const [room, setRoom] = useState<RoomData | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [loggedIn, setLoggedIn] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);

  // Available models + shared config
  const [allModels, setAllModels] = useState<{ id: string; model: string }[]>([]);
  const [sharedModels, setSharedModels] = useState<string[]>([]);

  // Create form
  const [creating, setCreating] = useState(false);
  const [formName, setFormName] = useState('My Room');
  const [formType, setFormType] = useState('private');
  const [formMaxConsumers, setFormMaxConsumers] = useState(4);
  const [formBudget, setFormBudget] = useState('');

  const fetchRoom = useCallback(async () => {
    try {
      const resp = await fetch('/api/rooms/mine');
      if (resp.status === 401) { setLoggedIn(false); return; }
      if (!resp.ok) return;
      const data = await resp.json();
      if (data.room) {
        setRoom(data.room);
        setMembers(data.members || []);
      } else {
        setRoom(null);
        setMembers([]);
      }
    } catch { /* ignore */ } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchRoom(); }, [fetchRoom]);

  // Fetch available models + room config (shared models)
  useEffect(() => {
    (async () => {
      try {
        const [modelsResp, configResp] = await Promise.all([
          fetch('/api/models'),
          fetch('/api/room-config'),
        ]);
        if (modelsResp.ok) {
          const data = await modelsResp.json();
          setAllModels(data.models || []);
        }
        if (configResp.ok) {
          const data = await configResp.json();
          setSharedModels(data.shared_models || []);
        }
      } catch { /* ignore */ }
    })();
  }, []);

  const toggleSharedModel = async (modelId: string) => {
    const newShared = sharedModels.includes(modelId)
      ? sharedModels.filter(id => id !== modelId)
      : [...sharedModels, modelId];
    setSharedModels(newShared);
    try {
      await fetch('/api/room-config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ shared_models: newShared }),
      });
    } catch { /* ignore */ }
  };

  const createRoom = async () => {
    setSaving(true); setError(null);
    try {
      // Get current instance id from remote config (proxied through /api/user/me won't have it)
      // Use the first available instance — the server knows its own instance_id
      const meResp = await fetch('/api/rooms/mine');
      if (meResp.status === 401) { setLoggedIn(false); return; }

      const resp = await fetch('/api/rooms/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: formName || 'My Room',
          room_type: formType,
          max_consumers: formMaxConsumers,
          token_budget_daily: formBudget ? parseInt(formBudget) : null,
        }),
      });
      const data = await resp.json();
      if (!resp.ok) { setError(data.error || 'Failed to create room'); return; }
      setCreating(false);
      fetchRoom();
    } catch (e: any) { setError(e.message); } finally { setSaving(false); }
  };

  const updateRoom = async (updates: Record<string, unknown>) => {
    setSaving(true); setError(null);
    try {
      const resp = await fetch('/api/rooms/', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updates),
      });
      if (!resp.ok) {
        const data = await resp.json();
        setError(data.error || 'Update failed');
        return;
      }
      fetchRoom();
    } catch (e: any) { setError(e.message); } finally { setSaving(false); }
  };

  const deleteRoom = async () => {
    if (!confirm('Delete your room? All members will be removed.')) return;
    setSaving(true);
    try {
      await fetch('/api/rooms/', { method: 'DELETE' });
      setRoom(null); setMembers([]);
    } catch { /* ignore */ } finally { setSaving(false); }
  };

  const regenerateInvite = async () => {
    if (!confirm('Regenerate invite link? The old link will stop working.')) return;
    setSaving(true);
    try {
      await fetch('/api/rooms/invite', { method: 'POST' });
      fetchRoom();
    } catch { /* ignore */ } finally { setSaving(false); }
  };

  const removeMember = async (userId: string) => {
    if (!confirm('Remove this member?')) return;
    try {
      await fetch(`/api/rooms/${room!.id}/members/${userId}`, { method: 'DELETE' });
      fetchRoom();
    } catch { /* ignore */ }
  };

  const copyInvite = () => {
    if (!room?.invite_token) return;
    navigator.clipboard.writeText(`https://linggen.dev/join/${room.invite_token}`).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  if (loading) {
    return <div className="text-center py-12 text-slate-400">Loading...</div>;
  }

  if (!loggedIn) {
    return (
      <div className="text-center py-12 space-y-3">
        <p className="text-slate-500">Sign in to linggen.dev to manage your proxy room.</p>
        <p className="text-xs text-slate-400">Click the avatar in the top bar to sign in.</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-bold text-slate-900 dark:text-white">Proxy Room</h2>
        <p className="text-xs text-slate-500 mt-1">Share your AI models with others through a proxy room.</p>
      </div>

      {error && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
          <p className="text-xs text-red-500">{error}</p>
        </div>
      )}

      {/* No room — show create */}
      {!room && !creating && (
        <div className="p-6 border border-dashed border-slate-300 dark:border-white/10 rounded-xl text-center space-y-3">
          <Users size={24} className="mx-auto text-slate-400" />
          <p className="text-sm text-slate-500">No proxy room yet.</p>
          <button
            onClick={() => setCreating(true)}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg transition-colors"
          >
            Create Room
          </button>
        </div>
      )}

      {/* Create form */}
      {creating && !room && (
        <div className="p-5 bg-slate-50 dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-[11px] font-bold text-slate-500 mb-1">Room Name</label>
              <input
                value={formName} onChange={e => setFormName(e.target.value)}
                className="w-full px-3 py-2 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-sm"
              />
            </div>
            <div>
              <label className="block text-[11px] font-bold text-slate-500 mb-1">Type</label>
              <select
                value={formType} onChange={e => setFormType(e.target.value)}
                className="w-full px-3 py-2 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-sm"
              >
                <option value="private">Private (invite only)</option>
                <option value="public">Public (anyone can join)</option>
              </select>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-[11px] font-bold text-slate-500 mb-1">Max Consumers</label>
              <select
                value={formMaxConsumers} onChange={e => setFormMaxConsumers(parseInt(e.target.value))}
                className="w-full px-3 py-2 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-sm"
              >
                {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
              </select>
            </div>
            <div>
              <label className="block text-[11px] font-bold text-slate-500 mb-1">Daily Token Budget</label>
              <input
                type="number" placeholder="Unlimited"
                value={formBudget} onChange={e => setFormBudget(e.target.value)}
                className="w-full px-3 py-2 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-sm"
              />
            </div>
          </div>
          <div className="flex gap-2">
            <button
              onClick={createRoom} disabled={saving}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg disabled:opacity-50"
            >
              {saving ? 'Creating...' : 'Create'}
            </button>
            <button
              onClick={() => { setCreating(false); setError(null); }}
              className="px-4 py-2 text-xs text-slate-500 hover:text-slate-700 dark:hover:text-slate-300"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Room card */}
      {room && (
        <div className="space-y-4">
          {/* Status header */}
          <div className="p-4 bg-slate-50 dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl space-y-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <h3 className="font-bold text-slate-900 dark:text-white">{room.name}</h3>
                <span className={`px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded ${
                  room.room_type === 'public'
                    ? 'bg-blue-500/10 text-blue-500'
                    : 'bg-slate-200 dark:bg-white/10 text-slate-500'
                }`}>
                  {room.room_type}
                </span>
              </div>
              <div className={`flex items-center gap-1.5 text-[11px] font-medium ${room.online ? 'text-green-500' : 'text-slate-400'}`}>
                <div className={`w-1.5 h-1.5 rounded-full ${room.online ? 'bg-green-500' : 'bg-slate-400'}`} />
                {room.online ? 'Online' : 'Offline'}
              </div>
            </div>

            {/* Settings */}
            <div className="grid grid-cols-3 gap-3">
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Type</label>
                <select
                  value={room.room_type}
                  onChange={e => updateRoom({ room_type: e.target.value })}
                  className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs"
                >
                  <option value="private">Private</option>
                  <option value="public">Public</option>
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Max Consumers</label>
                <select
                  value={room.max_consumers}
                  onChange={e => updateRoom({ max_consumers: parseInt(e.target.value) })}
                  className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs"
                >
                  {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Daily Budget</label>
                <input
                  type="number" placeholder="Unlimited"
                  defaultValue={room.token_budget_daily || ''}
                  onBlur={e => {
                    const val = e.target.value ? parseInt(e.target.value) : null;
                    if (val !== room.token_budget_daily) updateRoom({ token_budget_daily: val });
                  }}
                  className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs"
                />
              </div>
            </div>

            {/* Invite link */}
            {room.room_type === 'private' && room.invite_token && (
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Invite Link</label>
                <div className="flex items-center gap-2">
                  <input
                    readOnly
                    value={`https://linggen.dev/join/${room.invite_token}`}
                    className="flex-1 px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-[11px] font-mono text-slate-500"
                  />
                  <button onClick={copyInvite} className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors">
                    {copied ? <Check size={14} className="text-green-500" /> : <Copy size={14} className="text-slate-400" />}
                  </button>
                  <button onClick={regenerateInvite} disabled={saving} className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors">
                    <RefreshCw size={14} className="text-slate-400" />
                  </button>
                </div>
              </div>
            )}

            {/* Token usage */}
            {room.token_budget_daily != null && (
              <div>
                <div className="flex items-center justify-between text-[10px] mb-1">
                  <span className="text-slate-400">Token usage today</span>
                  <span className="font-mono text-slate-500">
                    {(room.tokens_used_today || 0).toLocaleString()} / {room.token_budget_daily.toLocaleString()}
                  </span>
                </div>
                <div className="w-full h-1 bg-slate-200 dark:bg-white/10 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-500 rounded-full"
                    style={{ width: `${Math.min(100, ((room.tokens_used_today || 0) / room.token_budget_daily) * 100)}%` }}
                  />
                </div>
              </div>
            )}
          </div>

          {/* Shared Models */}
          <div className="p-4 bg-slate-50 dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl space-y-3">
            <h4 className="text-[11px] font-bold text-slate-500">Shared Models</h4>
            <p className="text-[10px] text-slate-400">Select which models consumers can use through your room.</p>
            {allModels.length === 0 ? (
              <p className="text-xs text-slate-400">No models configured.</p>
            ) : (
              <div className="space-y-1.5">
                {allModels.map(m => (
                  <label key={m.id} className="flex items-center gap-2 py-1 px-2 rounded-lg hover:bg-white dark:hover:bg-white/5 cursor-pointer transition-colors">
                    <input
                      type="checkbox"
                      checked={sharedModels.includes(m.id)}
                      onChange={() => toggleSharedModel(m.id)}
                      className="accent-blue-500"
                    />
                    <span className="text-xs text-slate-700 dark:text-slate-300">{m.id}</span>
                    <span className="text-[9px] text-slate-400 font-mono">{m.model}</span>
                  </label>
                ))}
              </div>
            )}
            {sharedModels.length === 0 && allModels.length > 0 && (
              <p className="text-[10px] text-amber-500">No models shared. Consumers won't be able to use inference.</p>
            )}
          </div>

          {/* Members */}
          <div className="p-4 bg-slate-50 dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl space-y-3">
            <h4 className="text-[11px] font-bold text-slate-500">Members ({members.length}/{room.max_consumers})</h4>
            {members.length === 0 ? (
              <p className="text-xs text-slate-400">No members yet. Share your invite link to get started.</p>
            ) : (
              <div className="space-y-1.5">
                {members.map(m => (
                  <div key={m.user_id} className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-white dark:hover:bg-white/5 transition-colors">
                    <div className="flex items-center gap-2">
                      {m.avatar_url ? (
                        <img src={m.avatar_url} alt="" className="w-5 h-5 rounded-full" />
                      ) : (
                        <div className="w-5 h-5 rounded-full bg-blue-500/10 text-blue-500 text-[9px] font-bold flex items-center justify-center">
                          {(m.display_name || '?')[0].toUpperCase()}
                        </div>
                      )}
                      <span className="text-xs text-slate-700 dark:text-slate-300">{m.display_name}</span>
                      <span className="text-[9px] text-slate-400 font-mono">{m.consumer_type}</span>
                    </div>
                    <button onClick={() => removeMember(m.user_id)} className="p-1 text-slate-300 hover:text-red-500 transition-colors">
                      <Trash2 size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Actions */}
          <div className="flex items-center justify-between">
            <a
              href="https://linggen.dev/app"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-xs text-blue-500 hover:text-blue-600 font-medium"
            >
              Manage on linggen.dev <ExternalLink size={12} />
            </a>
            <button
              onClick={deleteRoom} disabled={saving}
              className="text-xs text-red-500 hover:text-red-600 font-medium"
            >
              Delete Room
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
