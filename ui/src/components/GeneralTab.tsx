import React from 'react';
import { Plus, Trash2 } from 'lucide-react';
import type { AppConfig } from '../types';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[10px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

export const GeneralTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  const addAgentRef = () => {
    onChange({ ...config, agents: [...config.agents, { id: '', spec_path: '', model: null }] });
  };

  const removeAgentRef = (index: number) => {
    onChange({ ...config, agents: config.agents.filter((_, i) => i !== index) });
  };

  const updateAgentRef = (index: number, field: string, value: string | null) => {
    const agents = [...config.agents];
    agents[index] = { ...agents[index], [field]: value };
    onChange({ ...config, agents });
  };

  return (
    <div className="space-y-6">
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
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, max_iters: parseInt(e.target.value) || 1 } })}
            />
          </div>
          <div>
            <label className={labelCls}>Write Safety Mode</label>
            <select
              className={inputCls}
              value={config.agent.write_safety_mode}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, write_safety_mode: e.target.value } })}
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
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, prompt_loop_breaker: e.target.value || null } })}
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
            onChange={(e) => onChange({ ...config, server: { ...config.server, port: parseInt(e.target.value) || 8080 } })}
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
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, level: e.target.value || null } })}
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
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, directory: e.target.value || null } })}
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
              onChange={(e) => onChange({ ...config, logging: { ...config.logging, retention_days: e.target.value ? parseInt(e.target.value) : null } })}
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
  );
};
