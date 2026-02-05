import React, { useState } from 'react';
import { MessageSquare, Copy, Eraser, Plus, Send, Activity } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, ChatMessage, SessionInfo, SkillInfo } from '../types';

export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  chatEndRef: React.RefObject<HTMLDivElement | null>;
  copyChat: () => void;
  copyChatStatus: 'idle' | 'copied' | 'error';
  clearChat: () => void;
  createSession: () => void;
  removeSession: (id: string) => void;
  sessions: SessionInfo[];
  activeSessionId: string | null;
  setActiveSessionId: (value: string | null) => void;
  selectedAgent: 'lead' | 'coder';
  setSelectedAgent: (value: 'lead' | 'coder') => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  onSendMessage: (message: string) => void;
}> = ({
  chatMessages,
  chatEndRef,
  copyChat,
  copyChatStatus,
  clearChat,
  createSession,
  removeSession,
  sessions,
  activeSessionId,
  setActiveSessionId,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  onSendMessage,
}) => {
  const [chatInput, setChatInput] = useState('');
  const [showSkillDropdown, setShowSkillDropdown] = useState(false);
  const [skillFilter, setSkillFilter] = useState('');
  const [showAgentDropdown, setShowAgentDropdown] = useState(false);
  const [agentFilter, setAgentFilter] = useState('');
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);

  const send = () => {
    if (!chatInput.trim()) return;
    const userMessage = chatInput;
    setChatInput('');
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);
    onSendMessage(userMessage);
  };

  const buildSkillSuggestions = () => {
    const suggestions: {
      key: string;
      label: string;
      description?: string;
      apply: () => void;
    }[] = [];

    const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));

    if ('mode'.includes(skillFilter)) {
      suggestions.push({
        key: 'cmd-mode',
        label: '/mode',
        description: 'Switch between chat and auto.',
        apply: () => {
          setChatInput(`${beforeSlash}/mode `);
          setSkillFilter('mode');
          setShowSkillDropdown(true);
        },
      });
    }

    if (skillFilter.startsWith('mode')) {
      [
        { cmd: '/mode chat', desc: 'Plain-text answers (summaries, explanations).' },
        { cmd: '/mode auto', desc: 'Structured planning responses (user stories + criteria).' },
      ].forEach((item) => {
        suggestions.push({
          key: item.cmd,
          label: item.cmd,
          description: item.desc,
          apply: () => {
            setChatInput(`${item.cmd} `);
            setShowSkillDropdown(false);
          },
        });
      });
    }

    skills
      .filter(
        (skill) =>
          skill.name.toLowerCase().includes(skillFilter) ||
          skill.description.toLowerCase().includes(skillFilter)
      )
      .forEach((skill) => {
        suggestions.push({
          key: `skill-${skill.name}`,
          label: `/${skill.name}`,
          description: skill.description,
          apply: () => {
            setChatInput(`${beforeSlash}/${skill.name} `);
            setShowSkillDropdown(false);
          },
        });
      });

    return suggestions;
  };

  return (
    <section className="h-full flex flex-col bg-white dark:bg-[#0f0f0f] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm overflow-hidden min-h-0">
      <div className="p-4 border-b border-slate-200 dark:border-white/5 flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500 flex items-center gap-2">
            <MessageSquare size={14} /> Unified Chat
          </h2>
          <div className="flex items-center gap-2">
            <button
              onClick={copyChat}
              className={cn(
                'p-1.5 rounded-lg transition-colors text-slate-500',
                copyChatStatus === 'copied'
                  ? 'bg-green-500/10 text-green-600'
                  : copyChatStatus === 'error'
                    ? 'bg-red-500/10 text-red-500'
                    : 'hover:bg-slate-100 dark:hover:bg-white/5'
              )}
              title={
                copyChatStatus === 'copied'
                  ? 'Copied'
                  : copyChatStatus === 'error'
                    ? 'Copy failed'
                    : 'Copy Chat'
              }
            >
              <Copy size={16} />
            </button>
            <button
              onClick={clearChat}
              className="p-1.5 hover:bg-red-500/10 hover:text-red-500 rounded-lg text-slate-500 transition-colors"
              title="Clear Chat"
            >
              <Eraser size={16} />
            </button>
            <button
              onClick={createSession}
              className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-blue-500 transition-colors"
              title="New Session"
            >
              <Plus size={16} />
            </button>
            <select
              value={selectedAgent}
              onChange={(e: any) => setSelectedAgent(e.target.value)}
              className="text-[10px] bg-slate-100 dark:bg-white/5 border-none rounded px-2 py-1 outline-none"
            >
              <option value="lead">Lead Agent</option>
              <option value="coder">Coder Agent</option>
            </select>
          </div>
        </div>

        <div className="flex flex-col gap-1">
          <select
            value={activeSessionId || ''}
            onChange={(e) => setActiveSessionId(e.target.value || null)}
            className="text-[10px] bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none w-full"
          >
            <option value="">Default Session</option>
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {s.title}
              </option>
            ))}
          </select>
          {activeSessionId && (
            <button onClick={() => removeSession(activeSessionId)} className="text-[8px] text-red-500 hover:underline text-right">
              Delete Session
            </button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-scroll p-4 flex flex-col gap-4 custom-scrollbar min-h-0">
        {chatMessages.map((msg, i) => (
          <div
            key={i}
            className={cn('flex flex-col gap-1 max-w-[95%]', msg.role === 'user' ? 'self-end items-end' : 'self-start items-start')}
          >
            <div className="flex items-center gap-1 px-1">
              <span className="text-[9px] font-bold uppercase tracking-tighter text-slate-500">
                {msg.from || msg.role} {msg.to ? `→ ${msg.to}` : ''}
              </span>
            </div>
            <div
              className={cn(
                'px-3 py-2 rounded-xl text-xs leading-relaxed shadow-sm',
                msg.role === 'user'
                  ? 'bg-blue-600 text-white rounded-tr-none'
                  : msg.from === 'lead' && msg.to === 'coder'
                    ? 'bg-amber-500/10 border border-amber-500/20 text-amber-200 italic'
                    : 'bg-slate-100 dark:bg-white/5 text-slate-800 dark:text-slate-200 border border-slate-200 dark:border-white/10 rounded-tl-none'
              )}
            >
              {(() => {
                if (msg.role === 'user') return msg.text;
                try {
                  const parsed = JSON.parse(msg.text);
                  if (parsed.type === 'ask' && parsed.question) {
                    return parsed.question;
                  }
                  if (parsed.type === 'tool' && parsed.tool) {
                    return (
                      <div className="flex items-center gap-2 text-blue-500 italic">
                        <Activity size={12} className="animate-pulse" />
                        <span>Using tool: {parsed.tool}...</span>
                      </div>
                    );
                  }
                  if (parsed.type === 'finalize_task' && parsed.packet) {
                    const packet = parsed.packet;
                    const userStories: string[] = Array.isArray(packet.user_stories) ? packet.user_stories : [];
                    const criteria: string[] = Array.isArray(packet.acceptance_criteria)
                      ? packet.acceptance_criteria
                      : [];
                    return (
                      <div className="space-y-2">
                        <div className="font-bold text-blue-500">Task Finalized: {packet.title}</div>
                        {userStories.length > 0 && (
                          <div className="space-y-1 text-[11px]">
                            <div className="uppercase tracking-wider text-[9px] text-slate-500">User Stories</div>
                            {userStories.map((story: string, idx: number) => (
                              <div key={idx} className="text-[11px] opacity-90">
                                - {story}
                              </div>
                            ))}
                          </div>
                        )}
                        {criteria.length > 0 && (
                          <div className="space-y-1 text-[11px]">
                            <div className="uppercase tracking-wider text-[9px] text-slate-500">Acceptance Criteria</div>
                            {criteria.map((crit: string, idx: number) => (
                              <div key={idx} className="text-[11px] opacity-90">
                                - {crit}
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    );
                  }
                  return msg.text;
                } catch (e) {
                  return msg.text;
                }
              })()}
              {msg.isGenerating && <span className="inline-block w-1.5 h-3.5 bg-blue-500 ml-1 animate-pulse align-middle" />}
            </div>
            <span className="text-[8px] text-slate-500 px-1">{msg.timestamp}</span>
          </div>
        ))}
        <div ref={chatEndRef} />
      </div>

      <div className="p-4 border-t border-slate-200 dark:border-white/5 space-y-3 bg-slate-50 dark:bg-white/[0.02]">
        <div className="flex gap-2 bg-slate-100 dark:bg-white/5 p-1 rounded-xl border border-slate-200 dark:border-white/10 relative">
          {showSkillDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto z-[70]">
              <div className="px-3 py-2 text-[10px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                Type to filter skills • Press Enter to send
              </div>
              {(() => {
                const suggestions = buildSkillSuggestions();
                return suggestions.map((item, idx) => (
                  <button
                    key={item.key}
                    onClick={item.apply}
                    className={cn(
                      'w-full px-3 py-2 text-left text-xs border-b border-slate-200 dark:border-white/5 last:border-none',
                      idx === selectedSuggestionIndex
                        ? 'bg-blue-500/10 text-blue-600'
                        : 'hover:bg-slate-100 dark:hover:bg-white/5'
                    )}
                  >
                    <div className="font-bold text-blue-500">{item.label}</div>
                    {item.description && <div className="text-slate-500 text-[10px]">{item.description}</div>}
                  </button>
                ));
              })()}
              {buildSkillSuggestions().length === 0 && (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching skills found</div>
              )}
            </div>
          )}
          {showAgentDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
              {agents
                .filter((agent) => agent.name.toLowerCase().includes(agentFilter))
                .map((agent) => (
                  <button
                    key={agent.name}
                    onClick={() => {
                      const beforeAt = chatInput.substring(0, chatInput.lastIndexOf('@'));
                      const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                      setChatInput(`${beforeAt}@${label} `);
                      setShowAgentDropdown(false);
                      setSelectedAgent(agent.name.toLowerCase() as 'lead' | 'coder');
                    }}
                    className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <div className="font-bold text-purple-500">@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                    <div className="text-slate-500 text-[10px]">{agent.description}</div>
                  </button>
                ))}
            </div>
          )}
          <input
            value={chatInput}
            onChange={(e) => {
              const val = e.target.value;
              setChatInput(val);
              if (val.includes('/') && !val.includes(' ', val.lastIndexOf('/'))) {
                setSkillFilter(val.substring(val.lastIndexOf('/') + 1).toLowerCase());
                setShowSkillDropdown(true);
                setShowAgentDropdown(false);
                setSelectedSuggestionIndex(0);
              } else if (val.includes('@') && !val.includes(' ', val.lastIndexOf('@'))) {
                setAgentFilter(val.substring(val.lastIndexOf('@') + 1).toLowerCase());
                setShowAgentDropdown(true);
                setShowSkillDropdown(false);
              } else {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
              }
            }}
            onKeyDown={(e) => {
              const suggestions = showSkillDropdown ? buildSkillSuggestions() : [];
              if (showSkillDropdown && suggestions.length > 0 && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
                e.preventDefault();
                const delta = e.key === 'ArrowDown' ? 1 : -1;
                setSelectedSuggestionIndex((prev) => (prev + delta + suggestions.length) % suggestions.length);
                return;
              }
              if (showSkillDropdown && suggestions.length > 0 && e.key === 'Enter') {
                e.preventDefault();
                suggestions[selectedSuggestionIndex]?.apply();
                return;
              }
              if (e.key === 'Enter' && !showSkillDropdown && !showAgentDropdown) send();
              if (e.key === 'Escape') {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
              }
            }}
            placeholder="Message... (use / for skills, @ for agents)"
            className="flex-1 bg-transparent border-none px-3 py-2 text-xs outline-none"
          />
          <button onClick={send} className="w-8 h-8 rounded-lg bg-blue-600 text-white flex items-center justify-center shadow-lg shadow-blue-600/20">
            <Send size={14} />
          </button>
        </div>
      </div>
    </section>
  );
};
