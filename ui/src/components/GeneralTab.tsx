import React from 'react';
import type { AppConfig } from '../types';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

export const GeneralTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
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
          <div>
            <label className={labelCls}>Default Permission Mode</label>
            <select
              className={inputCls}
              value={config.agent.tool_permission_mode}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, tool_permission_mode: e.target.value } })}
            >
              <option value="ask">read (default)</option>
              <option value="accept_edits">edit</option>
              <option value="auto">admin</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-0.5">Default mode for new sessions. Per-session mode can be changed in the chat header.</p>
          </div>
          <div>
            <label className={labelCls}>Default Session Policy</label>
            <select
              className={inputCls}
              value={config.agent.default_policy || 'interactive'}
              onChange={(e) => onChange({ ...config, agent: { ...config.agent, default_policy: e.target.value === 'interactive' ? null : e.target.value } })}
            >
              <option value="interactive">interactive — ask me (default)</option>
              <option value="strict">strict — silently deny out-of-scope</option>
              <option value="trusted">trusted — silently allow (⚠ unsafe)</option>
            </select>
            <p className="text-[11px] text-slate-400 mt-1 leading-relaxed">
              What happens when the agent tries to touch a path outside the session&apos;s grants, or runs a command in the config&apos;s <code className="text-[10px] bg-slate-100 dark:bg-white/5 px-1 rounded">ask</code> list (e.g.&nbsp;<code className="text-[10px] bg-slate-100 dark:bg-white/5 px-1 rounded">git push</code>):
            </p>
            <ul className="text-[11px] text-slate-400 mt-1 space-y-1 list-disc pl-4">
              <li><b>interactive</b> — you see a permission prompt and decide (Allow / Switch mode / Deny). Default for chats. Safe, but can be noisy.</li>
              <li><b>strict</b> — no prompt. The action is blocked and the agent receives an error (&quot;denied by session policy&quot;) so it can try something else or report back. Best for autonomous runs (missions, skills).</li>
              <li><b>trusted</b> — no prompt. Out-of-scope actions are auto-allowed; <code className="text-[10px] bg-slate-100 dark:bg-white/5 px-1 rounded">ask</code>-list commands are still blocked (silent deny). Use only when you trust the agent with filesystem access but still want the config&apos;s blocklist respected.</li>
            </ul>
            <p className="text-[11px] text-slate-500 mt-1">
              Hard <code className="text-[10px] bg-slate-100 dark:bg-white/5 px-1 rounded">deny</code>-rules (e.g.&nbsp;<code className="text-[10px] bg-slate-100 dark:bg-white/5 px-1 rounded">rm -rf</code>) always block regardless of policy. Skills, missions, and consumer sessions override this with their own policy.
            </p>
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
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className={labelCls}>Host</label>
            <input
              className={inputCls}
              value={config.server.host}
              onChange={(e) => onChange({ ...config, server: { ...config.server, host: e.target.value } })}
              placeholder="127.0.0.1"
            />
            <p className="text-[11px] text-slate-400 mt-1">Use 0.0.0.0 to allow LAN access.</p>
          </div>
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
          </div>
        </div>
        <p className="text-[11px] text-amber-500 mt-2">Changing host or port requires a server restart to take effect.</p>
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
    </div>
  );
};
