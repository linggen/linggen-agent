import React, { useEffect, useRef, useState } from 'react';
import { MessageSquare, Copy, Eraser, Plus, Send } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { cn } from '../lib/cn';
import type { AgentInfo, ChatMessage, QueuedChatItem, SessionInfo, SkillInfo } from '../types';

let mermaidInstance: any = null;
let mermaidInitialized = false;

async function getMermaid() {
  if (!mermaidInstance) {
    const module = await import('mermaid');
    mermaidInstance = module.default;
  }
  if (!mermaidInitialized) {
    mermaidInstance.initialize({
      startOnLoad: false,
      securityLevel: 'loose',
      theme: 'default',
    });
    mermaidInitialized = true;
  }
  return mermaidInstance;
}

const hashText = (text: string) => {
  let hash = 0;
  for (let i = 0; i < text.length; i += 1) {
    hash = (hash * 31 + text.charCodeAt(i)) | 0;
  }
  return Math.abs(hash).toString(36);
};

const MermaidBlock: React.FC<{ code: string }> = ({ code }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const idRef = useRef(`chat-mermaid-${hashText(code)}-${Math.random().toString(36).slice(2, 8)}`);

  useEffect(() => {
    let cancelled = false;

    const render = async () => {
      setError(null);
      if (!containerRef.current) return;
      containerRef.current.innerHTML = '<div class="markdown-mermaid-loading">Rendering Mermaid...</div>';
      try {
        const mermaid = await getMermaid();
        const { svg } = await mermaid.render(idRef.current, code.trim());
        if (!cancelled && containerRef.current) {
          containerRef.current.innerHTML = svg;
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    };

    render();
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <div className="markdown-mermaid-error">
        Mermaid error: {error}
      </div>
    );
  }
  return <div className="markdown-mermaid" ref={containerRef} />;
};

const MarkdownContent: React.FC<{ text: string }> = ({ text }) => (
  <div className="markdown-body break-words">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        pre: ({ children }) => <>{children}</>,
        code: ({ inline, className, children, ...props }: any) => {
          const raw = String(children ?? '').replace(/\n$/, '');
          const match = /language-([\w-]+)/.exec(className || '');
          const lang = match?.[1]?.toLowerCase();
          if (!inline && lang === 'mermaid') {
            return <MermaidBlock code={raw} />;
          }
          if (inline) {
            return <code {...props}>{children}</code>;
          }
          return (
            <pre>
              <code className={className} {...props}>{raw}</code>
            </pre>
          );
        },
      }}
    >
      {normalizeMarkdownish(text)}
    </ReactMarkdown>
  </div>
);

function normalizeMarkdownish(text: string): string {
  // Improve readability when model emits markdown tokens without proper newlines.
  return text
    .replace(/\s+(#{1,6}\s)/g, '\n\n$1')
    .replace(/\s+(\d+\.\s)/g, '\n$1')
    .replace(/\s+(-\s)/g, '\n$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
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
  onSendMessage: (message: string, targetAgent?: 'lead' | 'coder') => void;
}> = ({
  chatMessages,
  queuedMessages,
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
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const agentSelectRef = useRef<HTMLSelectElement | null>(null);

  const formatParty = (party?: string) => {
    if (!party) return '';
    return party.toUpperCase();
  };

  const resizeInput = () => {
    if (!inputRef.current) return;
    inputRef.current.style.height = '0px';
    const next = Math.min(inputRef.current.scrollHeight, 220);
    inputRef.current.style.height = `${next}px`;
  };

  useEffect(() => {
    resizeInput();
  }, [chatInput]);

  const send = () => {
    if (!chatInput.trim()) return;
    const userMessage = chatInput;
    setChatInput('');
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);

    const normalized = userMessage.trim().toLowerCase();
    let mentionAgent: 'lead' | 'coder' | undefined;
    if (normalized.startsWith('@coder ')) {
      mentionAgent = 'coder';
      setSelectedAgent('coder');
    } else if (normalized.startsWith('@lead ')) {
      mentionAgent = 'lead';
      setSelectedAgent('lead');
    }

    const dropdownAgent = agentSelectRef.current?.value as 'lead' | 'coder' | undefined;
    const targetAgent = mentionAgent || dropdownAgent || selectedAgent;
    onSendMessage(userMessage, targetAgent);
    window.setTimeout(resizeInput, 0);
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
      <div className="p-4 border-b border-slate-200 dark:border-white/5 flex flex-col gap-3 bg-gradient-to-r from-slate-50 to-white dark:from-[#121212] dark:to-[#0f0f0f]">
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
              value={activeSessionId || ''}
              onChange={(e) => setActiveSessionId(e.target.value || null)}
              className="text-[10px] bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none w-44"
            >
              <option value="">Default Session</option>
              {sessions.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.title}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="flex flex-col gap-1">
          {activeSessionId && (
            <button onClick={() => removeSession(activeSessionId)} className="text-[8px] text-red-500 hover:underline text-right">
              Delete Session
            </button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-scroll p-4 flex flex-col gap-5 custom-scrollbar min-h-0">
        {chatMessages.length === 0 && (
          <div className="self-center mt-12 max-w-md text-center">
            <div className="text-sm font-semibold text-slate-600 dark:text-slate-300">Start a conversation</div>
            <div className="mt-2 text-xs text-slate-500">
              Ask for edits, code review, or use commands like <code>/mode chat</code>.
            </div>
          </div>
        )}
        {chatMessages.map((msg, i) => {
          const key = `${msg.timestamp}-${i}-${msg.from || msg.role}-${msg.text.slice(0, 24)}`;
          const isUser = msg.role === 'user';
          const isStatusLine = msg.text === 'Thinking...' || msg.text.startsWith('Calling tool:');
          const from = msg.from || msg.role;
          const to = msg.to || '';
          return (
          <div
            key={key}
            className={cn('flex flex-col gap-1 max-w-[90%]', isUser ? 'self-end items-end' : 'self-start items-start')}
          >
            <div className="flex items-center gap-1 px-1">
              <span className="text-[9px] font-bold uppercase tracking-tighter text-slate-500">
                {formatParty(from)} {to ? `→ ${formatParty(to)}` : ''}
              </span>
            </div>
            <div
              className={cn(
                'px-3 py-2.5 rounded-2xl text-[13px] leading-relaxed shadow-sm',
                isUser
                  ? 'bg-blue-600 text-white rounded-tr-sm'
                  : msg.from === 'lead' && msg.to === 'coder'
                    ? 'bg-amber-500/10 border border-amber-500/20 text-amber-900 dark:text-amber-200 rounded-tl-sm'
                    : isStatusLine
                      ? 'bg-blue-50 border border-blue-200 text-blue-700 dark:bg-blue-500/10 dark:border-blue-400/20 dark:text-blue-300 rounded-tl-sm italic'
                      : 'bg-slate-100 dark:bg-white/5 text-slate-800 dark:text-slate-200 border border-slate-200 dark:border-white/10 rounded-tl-sm'
              )}
            >
              {(() => {
                if (isUser || isStatusLine) return msg.text;
                try {
                  const parsed = JSON.parse(msg.text);
                  if (parsed.type === 'ask' && parsed.question) {
                    return <MarkdownContent text={parsed.question} />;
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
                  return <MarkdownContent text={msg.text} />;
                } catch (e) {
                  return <MarkdownContent text={msg.text} />;
                }
              })()}
              {msg.isGenerating && <span className="inline-block w-1.5 h-3.5 bg-blue-500 ml-1 animate-pulse align-middle" />}
            </div>
            <span className="text-[10px] text-slate-500 px-1">{msg.timestamp}</span>
          </div>
        )})}
        <div ref={chatEndRef} />
      </div>

      <div className="p-4 border-t border-slate-200 dark:border-white/5 space-y-3 bg-slate-50 dark:bg-white/[0.02]">
        {queuedMessages.length > 0 && (
          <div className="rounded-lg border border-amber-300/50 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
            <div className="font-semibold">Queued messages ({queuedMessages.length})</div>
            <div className="mt-1 space-y-1">
              {queuedMessages.map((item) => (
                <div key={item.id} className="truncate">
                  [{item.agent_id}] {item.preview}
                </div>
              ))}
            </div>
          </div>
        )}
        <div className="flex gap-2 bg-white dark:bg-black/20 p-2 rounded-2xl border border-slate-300/80 dark:border-white/10 relative items-end shadow-sm">
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
          <select
            ref={agentSelectRef}
            value={selectedAgent}
            onChange={(e: any) => setSelectedAgent(e.target.value)}
            className="text-[11px] bg-slate-50 dark:bg-black/20 border border-slate-200 dark:border-white/10 rounded-xl px-2.5 py-2 outline-none"
          >
            <option value="lead">Lead</option>
            <option value="coder">Coder</option>
          </select>
          <textarea
            ref={inputRef}
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
              if (e.key === 'Enter' && !e.shiftKey && !showSkillDropdown && !showAgentDropdown) {
                e.preventDefault();
                send();
              }
              if (e.key === 'Escape') {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
              }
            }}
            placeholder="Message...  (/ for skills, @ for agents, Shift+Enter for newline)"
            rows={1}
            className="flex-1 bg-transparent border-none px-2 py-2 text-[13px] outline-none resize-none min-h-[40px] max-h-[220px] leading-5"
          />
          <button
            onClick={send}
            className="w-9 h-9 rounded-xl bg-blue-600 text-white flex items-center justify-center shadow-lg shadow-blue-600/20 hover:bg-blue-500 transition-colors"
            title="Send"
          >
            <Send size={14} />
          </button>
        </div>
      </div>
    </section>
  );
};
