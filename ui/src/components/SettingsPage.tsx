import React, { useCallback, useEffect, useState } from 'react';
import { ArrowLeft, Plus, Trash2 } from 'lucide-react';
import type { AppConfig, ModelConfigUI } from '../types';

const emptyModel = (): ModelConfigUI => ({
  id: '',
  provider: 'ollama',
  url: 'http://127.0.0.1:11434',
  model: '',
  api_key: null,
  keep_alive: null,
});

export const SettingsPage: React.FC<{
  onBack: () => void;
}> = ({ onBack }) => {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [originalConfig, setOriginalConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const dirty = config !== null && originalConfig !== null && JSON.stringify(config) !== JSON.stringify(originalConfig);

  const fetchConfig = useCallback(async () => {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) {
        setError('Failed to load config');
        return;
      }
      const data: AppConfig = await resp.json();
      setConfig(data);
      setOriginalConfig(data);
      setError(null);
    } catch (e) {
      setError(`Failed to load config: ${e}`);
    }
  }, []);

  useEffect(() => { fetchConfig(); }, [fetchConfig]);

  const saveConfig = async () => {
    if (!config || saving) return;
    setSaving(true);
    setError(null);
    setSuccess(false);
    try {
      const resp = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      });
      if (!resp.ok) {
        const text = await resp.text();
        setError(text || 'Save failed');
        return;
      }
      setOriginalConfig(config);
      setSuccess(true);
      setTimeout(() => setSuccess(false), 2000);
    } catch (e) {
      setError(`Save failed: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const updateModel = (index: number, field: keyof ModelConfigUI, value: string | null) => {
    if (!config) return;
    const models = [...config.models];
    models[index] = { ...models[index], [field]: value };
    setConfig({ ...config, models });
  };

  const addModel = () => {
    if (!config) return;
    setConfig({ ...config, models: [...config.models, emptyModel()] });
  };

  const removeModel = (index: number) => {
    if (!config) return;
    setConfig({ ...config, models: config.models.filter((_, i) => i !== index) });
  };

  const addAgentRef = () => {
    if (!config) return;
    setConfig({ ...config, agents: [...config.agents, { id: '', spec_path: '', model: null }] });
  };

  const removeAgentRef = (index: number) => {
    if (!config) return;
    setConfig({ ...config, agents: config.agents.filter((_, i) => i !== index) });
  };

  const updateAgentRef = (index: number, field: string, value: string | null) => {
    if (!config) return;
    const agents = [...config.agents];
    agents[index] = { ...agents[index], [field]: value };
    setConfig({ ...config, agents });
  };

  if (!config) {
    return (
      <div className="flex items-center justify-center h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-500">
        {error ? <p className="text-red-500">{error}</p> : <p>Loading config...</p>}
      </div>
    );
  }

  const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
  const labelCls = 'text-[10px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
  const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {/* Top bar */}
      <header className="flex items-center justify-between px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md z-50">
        <button onClick={onBack} className="flex items-center gap-2 text-xs font-semibold text-slate-600 dark:text-slate-300 hover:text-blue-600 transition-colors">
          <ArrowLeft size={16} /> Back
        </button>
        <h1 className="text-sm font-bold tracking-tight">Settings</h1>
        <div className="flex items-center gap-3">
          {error && <span className="text-[10px] text-red-500 max-w-60 truncate">{error}</span>}
          {success && <span className="text-[10px] text-green-500">Saved</span>}
          <button
            onClick={saveConfig}
            disabled={!dirty || saving}
            className={`px-4 py-1.5 rounded-lg text-xs font-bold transition-colors ${
              dirty
                ? 'bg-blue-600 text-white hover:bg-blue-700 shadow-lg shadow-blue-600/20'
                : 'bg-slate-200 dark:bg-white/10 text-slate-400 cursor-not-allowed'
            }`}
          >
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-4xl mx-auto space-y-6">

          {/* Models */}
          <section className={sectionCls}>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">Models</h2>
              <button onClick={addModel} className="flex items-center gap-1 text-[10px] font-bold text-blue-600 hover:text-blue-700">
                <Plus size={12} /> Add Model
              </button>
            </div>
            <div className="space-y-4">
              {config.models.map((model, i) => (
                <div key={model.id || i} className="bg-slate-50 dark:bg-white/[0.02] rounded-lg p-4 border border-slate-100 dark:border-white/5 relative">
                  <button
                    onClick={() => removeModel(i)}
                    className="absolute top-3 right-3 p-1 text-slate-400 hover:text-red-500 transition-colors"
                    title="Remove model"
                  >
                    <Trash2 size={12} />
                  </button>
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <label className={labelCls}>ID</label>
                      <input className={inputCls} value={model.id} onChange={(e) => updateModel(i, 'id', e.target.value)} placeholder="e.g. local_ollama" />
                    </div>
                    <div>
                      <label className={labelCls}>Provider</label>
                      <select className={inputCls} value={model.provider} onChange={(e) => updateModel(i, 'provider', e.target.value)}>
                        <option value="ollama">ollama</option>
                        <option value="openai">openai</option>
                      </select>
                    </div>
                    <div>
                      <label className={labelCls}>URL</label>
                      <input className={inputCls} value={model.url} onChange={(e) => updateModel(i, 'url', e.target.value)} placeholder="http://127.0.0.1:11434" />
                    </div>
                    <div>
                      <label className={labelCls}>Model</label>
                      <input className={inputCls} value={model.model} onChange={(e) => updateModel(i, 'model', e.target.value)} placeholder="e.g. qwen3-coder" />
                    </div>
                    <div>
                      <label className={labelCls}>API Key</label>
                      <input className={inputCls} type="password" value={model.api_key || ''} onChange={(e) => updateModel(i, 'api_key', e.target.value || null)} placeholder="(optional)" />
                    </div>
                    <div>
                      <label className={labelCls}>Keep Alive</label>
                      <input className={inputCls} value={model.keep_alive || ''} onChange={(e) => updateModel(i, 'keep_alive', e.target.value || null)} placeholder="e.g. 30m" />
                    </div>
                  </div>
                </div>
              ))}
              {config.models.length === 0 && (
                <p className="text-xs text-slate-400 text-center py-4">No models configured. Add at least one.</p>
              )}
            </div>
          </section>

          {/* Agent Settings */}
          <section className={sectionCls}>
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Agent Settings</h2>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className={labelCls}>Max Iterations</label>
                <input
                  className={inputCls}
                  type="number"
                  min={1}
                  value={config.agent.max_iters}
                  onChange={(e) => setConfig({ ...config, agent: { ...config.agent, max_iters: parseInt(e.target.value) || 1 } })}
                />
              </div>
              <div>
                <label className={labelCls}>Write Safety Mode</label>
                <select
                  className={inputCls}
                  value={config.agent.write_safety_mode}
                  onChange={(e) => setConfig({ ...config, agent: { ...config.agent, write_safety_mode: e.target.value } })}
                >
                  <option value="strict">strict</option>
                  <option value="warn">warn</option>
                  <option value="off">off</option>
                </select>
              </div>
              <div className="col-span-2">
                <label className={labelCls}>Prompt Loop Breaker</label>
                <textarea
                  className={`${inputCls} min-h-[60px] resize-y`}
                  value={config.agent.prompt_loop_breaker || ''}
                  onChange={(e) => setConfig({ ...config, agent: { ...config.agent, prompt_loop_breaker: e.target.value || null } })}
                  placeholder="(optional) Custom prompt to break tool loops"
                />
              </div>
            </div>
          </section>

          {/* Server */}
          <section className={sectionCls}>
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Server</h2>
            <div>
              <label className={labelCls}>Port</label>
              <input
                className={inputCls}
                type="number"
                min={1}
                max={65535}
                value={config.server.port}
                onChange={(e) => setConfig({ ...config, server: { ...config.server, port: parseInt(e.target.value) || 8080 } })}
              />
              <p className="text-[10px] text-amber-500 mt-1">Changing the port requires a server restart to take effect.</p>
            </div>
          </section>

          {/* Logging */}
          <section className={sectionCls}>
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Logging</h2>
            <div className="grid grid-cols-3 gap-4">
              <div>
                <label className={labelCls}>Level</label>
                <select
                  className={inputCls}
                  value={config.logging.level || ''}
                  onChange={(e) => setConfig({ ...config, logging: { ...config.logging, level: e.target.value || null } })}
                >
                  <option value="">Default</option>
                  <option value="trace">trace</option>
                  <option value="debug">debug</option>
                  <option value="info">info</option>
                  <option value="warn">warn</option>
                  <option value="error">error</option>
                </select>
              </div>
              <div>
                <label className={labelCls}>Directory</label>
                <input
                  className={inputCls}
                  value={config.logging.directory || ''}
                  onChange={(e) => setConfig({ ...config, logging: { ...config.logging, directory: e.target.value || null } })}
                  placeholder="(default)"
                />
              </div>
              <div>
                <label className={labelCls}>Retention Days</label>
                <input
                  className={inputCls}
                  type="number"
                  min={1}
                  value={config.logging.retention_days ?? ''}
                  onChange={(e) => setConfig({ ...config, logging: { ...config.logging, retention_days: e.target.value ? parseInt(e.target.value) : null } })}
                  placeholder="(default)"
                />
              </div>
            </div>
          </section>

          {/* Agent Spec Refs */}
          <section className={sectionCls}>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">Agent Spec Refs</h2>
              <button onClick={addAgentRef} className="flex items-center gap-1 text-[10px] font-bold text-blue-600 hover:text-blue-700">
                <Plus size={12} /> Add Ref
              </button>
            </div>
            <div className="space-y-3">
              {config.agents.map((ref_, i) => (
                <div key={ref_.id || i} className="flex items-center gap-3 bg-slate-50 dark:bg-white/[0.02] rounded-lg p-3 border border-slate-100 dark:border-white/5">
                  <div className="flex-1 grid grid-cols-3 gap-2">
                    <div>
                      <label className={labelCls}>ID</label>
                      <input className={inputCls} value={ref_.id} onChange={(e) => updateAgentRef(i, 'id', e.target.value)} placeholder="agent id" />
                    </div>
                    <div>
                      <label className={labelCls}>Spec Path</label>
                      <input className={inputCls} value={ref_.spec_path} onChange={(e) => updateAgentRef(i, 'spec_path', e.target.value)} placeholder="agents/lead.md" />
                    </div>
                    <div>
                      <label className={labelCls}>Model Override</label>
                      <input className={inputCls} value={ref_.model || ''} onChange={(e) => updateAgentRef(i, 'model', e.target.value || null)} placeholder="(inherit)" />
                    </div>
                  </div>
                  <button
                    onClick={() => removeAgentRef(i)}
                    className="p-1 text-slate-400 hover:text-red-500 transition-colors self-end mb-1"
                    title="Remove ref"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              ))}
              {config.agents.length === 0 && (
                <p className="text-xs text-slate-400 text-center py-4">No agent spec refs. Agents are discovered from the agents/ directory.</p>
              )}
            </div>
          </section>

        </div>
      </div>
    </div>
  );
};
