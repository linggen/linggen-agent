import React, { useCallback, useEffect, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import type { AppConfig, ModelConfigUI, OllamaPsResponse } from '../types';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[10px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

const emptyModel = (): ModelConfigUI => ({
  id: '',
  provider: 'ollama',
  url: 'http://127.0.0.1:11434',
  model: '',
  api_key: null,
  keep_alive: null,
});

const StatusDot: React.FC<{ status: 'connected' | 'disconnected' | 'na' }> = ({ status }) => {
  const cls =
    status === 'connected'
      ? 'bg-green-500'
      : status === 'disconnected'
        ? 'bg-red-500'
        : 'bg-slate-400';
  const label =
    status === 'connected'
      ? 'Connected'
      : status === 'disconnected'
        ? 'Disconnected'
        : 'N/A';
  return (
    <span className="inline-flex items-center gap-1.5" title={label}>
      <span className={`w-2 h-2 rounded-full ${cls}`} />
      <span className="text-[10px] text-slate-500">{label}</span>
    </span>
  );
};

export const ModelsTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);

  const fetchOllamaStatus = useCallback(async () => {
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) {
        setOllamaStatus(await resp.json());
      } else {
        setOllamaStatus(null);
      }
    } catch {
      setOllamaStatus(null);
    }
  }, []);

  useEffect(() => {
    fetchOllamaStatus();
    const timer = setInterval(fetchOllamaStatus, 5000);
    return () => clearInterval(timer);
  }, [fetchOllamaStatus]);

  const updateModel = (index: number, field: keyof ModelConfigUI, value: string | null) => {
    const models = [...config.models];
    models[index] = { ...models[index], [field]: value };
    onChange({ ...config, models });
  };

  const addModel = () => {
    onChange({ ...config, models: [...config.models, emptyModel()] });
  };

  const removeModel = (index: number) => {
    onChange({ ...config, models: config.models.filter((_, i) => i !== index) });
  };

  const getModelStatus = (model: ModelConfigUI): 'connected' | 'disconnected' | 'na' => {
    if (model.provider !== 'ollama') return 'na';
    if (!ollamaStatus?.models) return 'disconnected';
    const running = ollamaStatus.models.some(
      (m) => m.name === model.model || m.model === model.model
    );
    return running ? 'connected' : 'disconnected';
  };

  return (
    <div className="space-y-6">
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
              <div className="absolute top-3 right-3 flex items-center gap-3">
                <StatusDot status={getModelStatus(model)} />
                <button
                  onClick={() => removeModel(i)}
                  className="p-1 text-slate-400 hover:text-red-500 transition-colors"
                  title="Remove model"
                >
                  <Trash2 size={12} />
                </button>
              </div>
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
    </div>
  );
};
